# fkill

An interactive TUI process killer for macOS. Browse every process you own as a live
hierarchy, filter it instantly, and kill exactly what you meant to kill.

## Why

`kill` + `ps` + `grep` works until you have six node processes and one of them is
spinning. fkill shows the process **tree**, so when you search for a helper you also
see *who owns it* — and parents are always shown with the matches, styled yellow so
you know what's a direct hit vs. context.

htop is great for watching, but hard to *navigate* when your goal is just finding and
killing procs: every refresh re-sorts the list, so the process you were about to
highlight keeps jumping around under the cursor. fkill's list is stable — tree order,
never re-sorted — so what you're looking at stays where it is.

## Features

- **Hierarchical tree** — every process rendered under its real parent with
  `├──`/`└──` connectors
- **Live filtering** — multi-term AND, smart-case substring matching against the
  process name and pid (no path/args noise)
- **Parent visibility** — searching `renderer` shows the Chrome helper *and* its
  parent chain; direct matches are white, pulled-in parents are yellow
- **Live CPU% per row** — instantaneous rate computed from CPU-time deltas every
  refresh, color-coded (red ≥80%, yellow ≥25%) so a runaway process is obvious
- **Killable-only list** — non-root users only see processes they can actually kill;
  your own session chain (shell → terminal) is visible but locked
- **Selection view** — `ctrl+l` filters the list down to everything you've selected
  plus its parents, so you can review and deselect before pulling the trigger
- **Details pane** — full command line, pid/ppid, %cpu, %mem, uptime for the
  highlighted row

## Install

Requires the Rust toolchain ([rustup](https://rustup.rs)).

```sh
cargo install --git https://github.com/adibsaad/fkill
```

Or build from a clone:

```sh
git clone https://github.com/adibsaad/fkill && cd fkill
cargo build --release && cp target/release/fkill ~/.local/bin/
```

## Usage

```
fkill
```

| Key | Action |
| --- | --- |
| type | filter by name / pid (multi-word AND, smart-case) |
| `↑`/`↓` | move cursor |
| `tab` | toggle-select the highlighted process |
| `ctrl+l` | selection view — list only selected + parents, `tab` deselects |
| `enter` | kill all selected (confirm prompt) |
| `ctrl+u` | clear search |
| `ctrl+r` | refresh now (also auto-refreshes every 2s) |
| `esc` | back out of selection view / quit |

Anything in your own session's ancestry (fkill → shell → terminal) is rendered
struck-through and refused, so you can't nuke the terminal you're sitting in.

## Notes

- macOS only right now (`ps` flags are BSD-style; Linux needs a flag tweak)
- Killing uses `SIGKILL` after an explicit confirm; failures (e.g. race-lost pids)
  are reported per-process in the status line
