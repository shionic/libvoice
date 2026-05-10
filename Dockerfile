# Build stage
FROM rust:1.94-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev libfontconfig1-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build --release --locked \
    -p voiceanalyzerserver \
    -p voiceanalyzercli \
    -p telegrambot

# Runtime base stage
FROM debian:bookworm-slim AS runtime-base
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg fontconfig fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*
ENV RUST_LOG=info

# Server target
FROM runtime-base AS server
COPY --from=builder /app/target/release/voiceanalyzerserver /usr/local/bin/voiceanalyzerserver
EXPOSE 3000
ENTRYPOINT ["voiceanalyzerserver"]
CMD ["--bind", "0.0.0.0:3000"]

# CLI target
FROM runtime-base AS cli
COPY --from=builder /app/target/release/voiceanalyzercli /usr/local/bin/voiceanalyzercli
WORKDIR /workspace
ENTRYPOINT ["voiceanalyzercli"]
CMD ["--help"]

# Telegram Bot target
FROM runtime-base AS telegrambot
COPY --from=builder /app/target/release/telegrambot /usr/local/bin/telegrambot
ENTRYPOINT ["telegrambot"]

# All-in-one target (default)
FROM runtime-base AS full
COPY --from=builder /app/target/release/voiceanalyzerserver /usr/local/bin/voiceanalyzerserver
COPY --from=builder /app/target/release/voiceanalyzercli /usr/local/bin/voiceanalyzercli
COPY --from=builder /app/target/release/telegrambot /usr/local/bin/telegrambot
CMD ["voiceanalyzerserver", "--bind", "0.0.0.0:3000"]
