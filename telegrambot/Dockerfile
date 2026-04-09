FROM rust:1.94-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev libfontconfig1-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build --release --locked -p telegrambot

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg fontconfig fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*

ENV RUST_LOG=telegrambot=info,teloxide=info

COPY --from=builder /app/target/release/telegrambot /usr/local/bin/telegrambot
ENTRYPOINT ["telegrambot"]
