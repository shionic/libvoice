FROM rust:1.95-slim-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY libvoice/Cargo.toml libvoice/Cargo.toml
COPY libvoice-wasm/Cargo.toml libvoice-wasm/Cargo.toml
COPY telegrambot/Cargo.toml telegrambot/Cargo.toml
COPY voiceanalyzerserver/Cargo.toml voiceanalyzerserver/Cargo.toml
COPY voiceanalyzercli/Cargo.toml voiceanalyzercli/Cargo.toml

RUN mkdir -p libvoice/src libvoice-wasm/src telegrambot/src voiceanalyzerserver/src voiceanalyzercli/src
RUN printf 'pub fn placeholder() {}\n' > libvoice/src/lib.rs
RUN printf 'pub fn placeholder() {}\n' > libvoice-wasm/src/lib.rs
RUN printf 'fn main() {}\n' > telegrambot/src/main.rs
RUN printf 'fn main() {}\n' > voiceanalyzerserver/src/main.rs
RUN printf 'fn main() {}\n' > voiceanalyzercli/src/main.rs
RUN cargo build --release -p telegrambot

COPY libvoice libvoice
COPY libvoice-wasm libvoice-wasm
COPY telegrambot telegrambot
COPY voiceanalyzerserver voiceanalyzerserver
COPY voiceanalyzercli voiceanalyzercli

RUN cargo build --release -p telegrambot

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*

ENV RUST_LOG=telegrambot=info,teloxide=info

COPY --from=builder /app/target/release/telegrambot /usr/local/bin/telegrambot
ENTRYPOINT ["telegrambot"]
