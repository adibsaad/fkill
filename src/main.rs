use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::Path;
use std::process::Command as SysCommand;
use std::time::{Duration, Instant};

use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

#[derive(Clone)]
struct Proc {
    pid: i32,
    ppid: i32,
    user: String,
    cpu: String,
    mem: String,
    etime: String,
    command: String,
    name: String,
    own: String,
    search: String,
}

#[derive(Clone)]
struct Row {
    pid: i32,
    display: String,
    killable: bool,
    protected: bool,
}

enum Mode {
    Normal,
    Confirm(Vec<i32>),
}

struct App {
    procs: BTreeMap<i32, Proc>,
    children: HashMap<i32, Vec<i32>>,
    full_rows: Vec<(i32, String)>,
    visible: Vec<Row>,
    protected: HashSet<i32>,
    input: String,
    cursor: usize,
    cursor_pid: i32,
    selected: HashSet<i32>,
    message: String,
    mode: Mode,
}

fn proc_name(command: &str) -> String {
    let toks: Vec<&str> = command.split(' ').collect();
    let mut best = String::new();
    for i in 1..=toks.len() {
        let cand = toks[..i].join(" ");
        if Path::new(&cand).exists() {
            best = cand;
        } else if !best.is_empty() {
            break;
        }
    }
    if best.is_empty() {
        best = toks[0].to_string();
    }
    best.rsplit('/').next().unwrap_or(&best).to_string()
}

fn fetch() -> io::Result<BTreeMap<i32, Proc>> {
    let uid_out = SysCommand::new("id").arg("-u").output()?;
    let uid: u32 = String::from_utf8_lossy(&uid_out.stdout)
        .trim()
        .parse()
        .unwrap_or(u32::MAX);
    let is_root = uid == 0;
    let me = if is_root {
        String::new()
    } else {
        String::from_utf8_lossy(&SysCommand::new("id").arg("-un").output()?.stdout)
            .trim()
            .to_string()
    };
    let out = SysCommand::new("ps")
        .args(["-axo", "pid=,ppid=,user=,%cpu=,%mem=,etime=,command="])
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut procs = BTreeMap::new();
    for line in text.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() < 7 {
            continue;
        }
        let pid: i32 = match toks[0].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let user = toks[2].to_string();
        if !is_root && user != me {
            continue;
        }
        let ppid: i32 = toks[1].parse().unwrap_or(0);
        let command = toks[6..].join(" ").replace('\t', " ");
        let name = proc_name(&command);
        procs.insert(
            pid,
            Proc {
                pid,
                ppid,
                user: user.clone(),
                cpu: toks[3].to_string(),
                mem: toks[4].to_string(),
                etime: toks[5].to_string(),
                command,
                name: name.clone(),
                own: format!("{} {}", pid, name),
                search: String::new(),
            },
        );
    }
    Ok(procs)
}

fn build_children(procs: &BTreeMap<i32, Proc>) -> HashMap<i32, Vec<i32>> {
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for p in procs.values() {
        children.entry(p.ppid).or_default().push(p.pid);
    }
    children
}

fn protected_set(procs: &BTreeMap<i32, Proc>) -> HashSet<i32> {
    let mut protected = HashSet::new();
    let self_pid = std::process::id() as i32;
    let mut cur = self_pid;
    while procs.contains_key(&cur) && protected.insert(cur) {
        cur = procs[&cur].ppid;
    }
    protected
}

fn build_search(
    pid: i32,
    procs: &BTreeMap<i32, Proc>,
    children: &HashMap<i32, Vec<i32>>,
    cache: &mut HashMap<i32, String>,
    seen: &mut HashSet<i32>,
) -> String {
    if let Some(s) = cache.get(&pid) {
        return s.clone();
    }
    if !seen.insert(pid) {
        return String::new();
    }
    let mut parts = vec![procs[&pid].own.clone()];
    if let Some(kids) = children.get(&pid) {
        for c in kids {
            let s = build_search(*c, procs, children, cache, seen);
            if !s.is_empty() {
                parts.push(s);
            }
        }
    }
    let text = parts.join(" ");
    cache.insert(pid, text.clone());
    text
}

#[allow(clippy::too_many_arguments)]
fn walk_tree(
    pid: i32,
    depth: usize,
    prefix: &str,
    last: bool,
    procs: &BTreeMap<i32, Proc>,
    children: &HashMap<i32, Vec<i32>>,
    seen: &mut HashSet<i32>,
    out: &mut Vec<(i32, String)>,
) {
    if !seen.insert(pid) {
        return;
    }
    let p = &procs[&pid];
    let connector = if depth == 0 {
        ""
    } else if last {
        "└── "
    } else {
        "├── "
    };
    out.push((pid, format!("{}{}{}", prefix, connector, p.name)));
    if let Some(kids) = children.get(&pid) {
        for (i, child) in kids.iter().enumerate() {
            let child_prefix = if depth == 0 {
                String::new()
            } else {
                format!("{}{}", prefix, if last { "    " } else { "│   " })
            };
            walk_tree(
                *child,
                depth + 1,
                &child_prefix,
                i == kids.len() - 1,
                procs,
                children,
                seen,
                out,
            );
        }
    }
}

fn matches(text: &str, terms: &[String]) -> bool {
    terms.iter().all(|t| {
        if t.chars().any(|c| c.is_uppercase()) {
            text.contains(t.as_str())
        } else {
            text.to_lowercase().contains(&t.to_lowercase())
        }
    })
}

impl App {
    fn refresh(&mut self) -> io::Result<()> {
        let procs = fetch()?;
        let children = build_children(&procs);
        let protected = protected_set(&procs);
        let roots: Vec<i32> = procs
            .iter()
            .filter(|(_, p)| !procs.contains_key(&p.ppid))
            .map(|(pid, _)| *pid)
            .collect();
        let mut cache: HashMap<i32, String> = HashMap::new();
        let mut seen = HashSet::new();
        for root in &roots {
            build_search(*root, &procs, &children, &mut cache, &mut seen);
        }
        let mut full_rows = Vec::new();
        let mut seen2 = HashSet::new();
        for root in &roots {
            walk_tree(
                *root,
                0,
                "",
                true,
                &procs,
                &children,
                &mut seen2,
                &mut full_rows,
            );
        }
        self.procs = procs;
        self.children = children;
        self.protected = protected;
        self.full_rows = full_rows;
        for (pid, s) in cache {
            if let Some(p) = self.procs.get_mut(&pid) {
                p.search = s;
            }
        }
        self.rebuild_visible();
        Ok(())
    }

    fn terms(&self) -> Vec<String> {
        self.input.split_whitespace().map(|s| s.to_string()).collect()
    }

    fn rebuild_visible(&mut self) {
        let terms = self.terms();
        let mut visible = Vec::new();
        for (pid, display) in &self.full_rows {
            let p = match self.procs.get(pid) {
                Some(p) => p,
                None => continue,
            };
            let vis = terms.is_empty() || matches(&p.search, &terms);
            if !vis {
                continue;
            }
            let is_protected = self.protected.contains(pid);
            let killable = !is_protected && (terms.is_empty() || matches(&p.own, &terms));
            visible.push(Row {
                pid: *pid,
                display: display.clone(),
                killable,
                protected: is_protected,
            });
        }
        if let Some(i) = visible.iter().position(|r| r.pid == self.cursor_pid) {
            self.cursor = i;
        } else {
            self.cursor = visible.iter().position(|r| r.killable).unwrap_or(0);
            self.cursor_pid = visible.get(self.cursor).map(|r| r.pid).unwrap_or(0);
        }
        self.selected
            .retain(|p| self.procs.contains_key(p));
        self.visible = visible;
    }

    fn move_cursor(&mut self, delta: i32) {
        if self.visible.is_empty() {
            self.cursor = 0;
            self.cursor_pid = 0;
            return;
        }
        let len = self.visible.len() as i32;
        self.cursor = (self.cursor as i32 + delta).clamp(0, len - 1) as usize;
        self.cursor_pid = self.visible[self.cursor].pid;
    }

    fn toggle_select(&mut self) {
        if let Some(row) = self.visible.get(self.cursor) {
            if row.protected {
                self.message = "part of your own session — can't kill".into();
            } else {
                if self.selected.contains(&row.pid) {
                    self.selected.remove(&row.pid);
                } else {
                    self.selected.insert(row.pid);
                }
                self.message.clear();
            }
        }
    }

    fn kill_targets(&self) -> Vec<i32> {
        let mut targets: Vec<i32> = self
            .selected
            .iter()
            .copied()
            .filter(|p| !self.protected.contains(p) && self.procs.contains_key(p))
            .collect();
        if targets.is_empty() {
            if let Some(r) = self.visible.get(self.cursor) {
                if !r.protected {
                    targets.push(r.pid);
                }
            }
        }
        targets
    }

    fn do_kill(&mut self) {
        let targets = match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Confirm(t) => t,
            Mode::Normal => return,
        };
        let mut killed = Vec::new();
        let mut failed = Vec::new();
        for pid in &targets {
            match SysCommand::new("kill").arg("-9").arg(pid.to_string()).output() {
                Ok(o) if o.status.success() => killed.push(*pid),
                Ok(o) => failed.push(format!(
                    "{}: {}",
                    pid,
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => failed.push(format!("{}: {}", pid, e)),
            }
        }
        for pid in &killed {
            self.selected.remove(pid);
        }
        self.message = format!(
            "SIGKILLed {} process(es){}",
            killed.len(),
            if failed.is_empty() {
                String::new()
            } else {
                format!(" — failed: {}", failed.join("; "))
            }
        );
        let _ = self.refresh();
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(v[1])[1]
}

fn draw(f: &mut Frame, app: &mut App, list_state: &mut ListState) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(6),
        Constraint::Length(1),
    ])
    .split(area);

    let input_line = Line::from(vec![
        Span::styled("search> ", Style::default().fg(Color::Cyan)),
        Span::raw(app.input.clone()),
        Span::styled("█", Style::default().fg(Color::Cyan)),
    ]);
    f.render_widget(Paragraph::new(input_line), chunks[0]);

let items: Vec<ListItem> = app
            .visible
            .iter()
            .map(|r| {
                let style = if r.protected {
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::CROSSED_OUT)
                } else if r.killable {
                    Style::default()
                } else {
                    Style::default().fg(Color::Yellow)
                };
            let marker = if app.selected.contains(&r.pid) {
                "● "
            } else {
                "  "
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Green)),
                Span::styled(r.display.clone(), style),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_symbol("▌ ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(list, chunks[1], list_state);

    let preview_lines: Vec<Line> = match app.visible.get(app.cursor) {
        Some(row) => {
            let p = &app.procs[&row.pid];
            vec![
                Line::from(vec![
                    Span::styled(format!(" pid {} ", p.pid), Style::default().fg(Color::Yellow)),
                    Span::styled(format!("ppid {} ", p.ppid), Style::default().fg(Color::Yellow)),
                    Span::styled(format!("user {} ", p.user), Style::default().fg(Color::Yellow)),
                    Span::styled(format!("%cpu {} ", p.cpu), Style::default().fg(Color::Yellow)),
                    Span::styled(format!("%mem {} ", p.mem), Style::default().fg(Color::Yellow)),
                    Span::styled(format!("up {}", p.etime), Style::default().fg(Color::Yellow)),
                ]),
                Line::from(p.command.clone()),
            ]
        }
        None => vec![Line::from(" no process")],
    };
    let preview = Paragraph::new(preview_lines)
        .block(Block::default().borders(Borders::ALL).title(" details "))
        .wrap(Wrap { trim: false });
    f.render_widget(preview, chunks[2]);

    let mut status = String::new();
    if !app.message.is_empty() {
        status.push_str(&app.message);
        status.push_str("  ·  ");
    }
    status.push_str(&format!(
        "{} shown · {} selected  ·  ↑↓ move · tab select · enter kill · ctrl+u clear · ctrl+r refresh · esc quit",
        app.visible.len(),
        app.selected.len()
    ));
    f.render_widget(
        Paragraph::new(Span::styled(status, Style::default().fg(Color::DarkGray))),
        chunks[3],
    );

    if let Mode::Confirm(targets) = &app.mode {
        let rect = centered_rect(60, 40, area);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " confirm SIGKILL ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let list_height = inner.height.saturating_sub(1) as usize;
        let mut lines: Vec<Line> = Vec::new();
        for pid in targets.iter().take(list_height.saturating_sub(1)) {
            let name = app
                .procs
                .get(pid)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            lines.push(Line::from(format!("  {}  {}", pid, name)));
        }
        if targets.len() > list_height.saturating_sub(1) {
            lines.push(Line::from(format!(
                "  … +{} more",
                targets.len() - (list_height.saturating_sub(1))
            )));
        }
        let list_chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
        f.render_widget(Paragraph::new(lines), list_chunks[0]);
        f.render_widget(
            Paragraph::new(Span::styled(
                " [y] kill · any other key cancels",
                Style::default().fg(Color::Red),
            )),
            list_chunks[1],
        );
    }
}

fn run(mut app: App, mut terminal: Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut list_state = ListState::default();
    let mut last_refresh = Instant::now();
    loop {
        list_state.select(Some(app.cursor.min(app.visible.len().saturating_sub(1))));
        terminal.draw(|f| draw(f, &mut app, &mut list_state))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match &mut app.mode {
                        Mode::Normal => match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),
                            (KeyCode::Esc, _) => return Ok(()),
                            (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                                app.refresh()?;
                                last_refresh = Instant::now();
                            }
                            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                                app.input.clear();
                                app.rebuild_visible();
                            }
                            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                                app.input.push(c);
                                app.rebuild_visible();
                            }
                            (KeyCode::Backspace, _) => {
                                app.input.pop();
                                app.rebuild_visible();
                            }
                            (KeyCode::Up, _) => app.move_cursor(-1),
                            (KeyCode::Down, _) => app.move_cursor(1),
                            (KeyCode::Tab, _) => app.toggle_select(),
                            (KeyCode::Enter, _) => {
                                let targets = app.kill_targets();
                                if targets.is_empty() {
                                    app.message = "nothing selectable here".into();
                                } else {
                                    app.mode = Mode::Confirm(targets);
                                }
                            }
                            _ => {}
                        },
                        Mode::Confirm(_) => match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => app.do_kill(),
                            _ => app.mode = Mode::Normal,
                        },
                    }
                }
            }
        }
        if last_refresh.elapsed() >= Duration::from_secs(2) {
            app.refresh()?;
            last_refresh = Instant::now();
        }
    }
}

fn main() -> io::Result<()> {
    let mut app = App {
        procs: BTreeMap::new(),
        children: HashMap::new(),
        full_rows: Vec::new(),
        visible: Vec::new(),
        protected: HashSet::new(),
        input: String::new(),
        cursor: 0,
        cursor_pid: 0,
        selected: HashSet::new(),
        message: String::new(),
        mode: Mode::Normal,
    };
    app.refresh()?;

    if std::env::args().any(|a| a == "--dump") {
        if let Some(q) = std::env::args().nth(2) {
            app.input = q;
            app.rebuild_visible();
        }
        for r in &app.visible {
            let tag = if r.protected {
                "P"
            } else if r.killable {
                "K"
            } else {
                "s"
            };
            println!("{}\t{}\t{}", r.pid, tag, r.display);
        }
        eprintln!(
            "{} shown, {} selectable",
            app.visible.len(),
            app.visible.iter().filter(|r| r.killable).count()
        );
        return Ok(());
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        orig_hook(info);
    }));

    let res = run(app, terminal);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    res
}