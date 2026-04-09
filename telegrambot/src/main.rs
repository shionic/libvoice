mod audio;
mod input;
mod options;
mod report;

use audio::{analyze_samples, decode_audio_bytes};
use input::find_input_audio;
use options::{analyze_usage_hint, parse_analyze_options};
use report::format_report;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{Message, ParseMode};
use tokio::task;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    init_logging();
    let bot = Bot::from_env();
    info!("telegrambot started");

    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        let reply_chat_id = msg.chat.id;
        let command_text = msg.text().map(str::to_owned);
        let error_bot = bot.clone();
        if let Err(error) = handle_message(bot, msg).await {
            error!(%error, "request handling failed");
            if command_text
                .as_deref()
                .is_some_and(|text| text.starts_with("/analyze"))
            {
                let _ = error_bot
                    .send_message(
                        reply_chat_id,
                        format!(
                            "<b>Could not analyze that audio.</b>\n\n{}",
                            escape_html(&error)
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await;
            }
        }
        respond(())
    })
    .await;
}

async fn handle_message(bot: Bot, msg: Message) -> Result<(), String> {
    let Some(text) = msg.text() else {
        return Ok(());
    };
    if !text.starts_with("/analyze") {
        return Ok(());
    }

    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        command = text,
        "received analyze command"
    );

    let options = parse_analyze_options(text)?;
    let input = find_input_audio(&msg).ok_or_else(|| analyze_usage_hint().to_string())?;
    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        label = %input.label,
        "selected input audio"
    );

    bot.send_message(
        msg.chat.id,
        format!(
            "<b>Received</b> {}.\nDownloading and analyzing it now. This can take a few seconds.",
            escape_html(&input.label)
        ),
    )
    .parse_mode(ParseMode::Html)
    .await
    .map_err(|error| format!("failed to send progress message: {error}"))?;

    let telegram_file = bot
        .get_file(input.file_id.clone())
        .await
        .map_err(|error| format!("failed to fetch Telegram file metadata: {error}"))?;

    let mut bytes = Vec::new();
    bot.download_file(&telegram_file.path, &mut bytes)
        .await
        .map_err(|error| format!("failed to download Telegram file: {error}"))?;
    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        bytes = bytes.len(),
        telegram_path = %telegram_file.path,
        "downloaded telegram file"
    );

    let file_name = input.file_name.clone();
    let report_text = task::spawn_blocking(move || {
        let decoded = decode_audio_bytes(&bytes, file_name.as_deref())?;
        let report = analyze_samples(&decoded);
        Ok::<_, String>(format_report(&input.label, &decoded, &report, &options))
    })
    .await
    .map_err(|error| format!("analysis task failed: {error}"))??;
    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        report_len = report_text.len(),
        "analysis completed"
    );

    send_long_message(&bot, msg.chat.id, &report_text).await
}

async fn send_long_message(bot: &Bot, chat_id: ChatId, text: &str) -> Result<(), String> {
    const LIMIT: usize = 3500;
    if text.len() <= LIMIT {
        bot.send_message(chat_id, text.to_string())
            .parse_mode(ParseMode::Html)
            .await
            .map_err(|error| format!("failed to send analysis result: {error}"))?;
        return Ok(());
    }

    let mut current = String::new();
    for line in text.lines() {
        if !current.is_empty() && current.len() + line.len() + 1 > LIMIT {
            bot.send_message(chat_id, current.clone())
                .parse_mode(ParseMode::Html)
                .await
                .map_err(|error| format!("failed to send analysis chunk: {error}"))?;
            current.clear();
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        bot.send_message(chat_id, current)
            .parse_mode(ParseMode::Html)
            .await
            .map_err(|error| format!("failed to send final analysis chunk: {error}"))?;
    }

    Ok(())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("telegrambot=info,teloxide=info"));

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(true)
        .with_file(false)
        .with_line_number(false);

    if let Err(error) = builder.try_init() {
        warn!(%error, "logging was already initialized");
    }
}
