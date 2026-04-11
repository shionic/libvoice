# telegrambot

Telegram bot for `libvoice` analysis using `teloxide`.

## Requirements

- `TELOXIDE_TOKEN` environment variable with your bot token
- `ffmpeg` available in `PATH` for Telegram voice messages (`.ogg` / Opus)

## Run

```bash
cargo run -p telegrambot
```

## Usage

Reply to a voice message or audio file with:

```text
/analyze
```

Default features:

```text
+pitch +hnr +spectral
```

Enable or disable features explicitly:

```text
/analyze +formants
/analyze +energy -spectral
/analyze +spectrum
/analyze +all
/analyze -all +pitch +hnr
```

Analyze a specific time range:

```text
/analyze +from 20s +to 1m40s
/analyze +from 20s +dur 20s
```
