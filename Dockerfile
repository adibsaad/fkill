FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release

FROM debian:trixie-slim
RUN apt-get update -qq \
    && apt-get install -y -qq --no-install-recommends procps python3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/fkill /usr/local/bin/fkill
RUN ln -s /usr/bin/python3 /tmp/py_hog
ENTRYPOINT ["sleep", "infinity"]