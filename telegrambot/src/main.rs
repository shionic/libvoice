mod input;
mod voice;

use input::{InputAudio, find_input_audio};
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InputFile, InputMedia, InputMediaPhoto, Message, ParseMode, ThreadId};
use tokio::task;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use voice::prepare_voice_upload;
use voiceanalysis::{GraphImage, analyze_audio_bytes, analyze_usage_hint, parse_analyze_options};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BotCommand {
    Analyze,
    Voice,
}

#[tokio::main]
async fn main() {
    init_logging();
    let bot = Bot::from_env();
    info!("telegrambot started");

    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        let reply_chat_id = msg.chat.id;
        let reply_thread_id = msg.thread_id;
        let command_text = msg.text().map(str::to_owned);
        let command = command_text.as_deref().and_then(parse_command);
        let error_bot = bot.clone();
        if let Err(error) = handle_message(bot, msg).await {
            error!(%error, "request handling failed");
            if let Some(command) = command {
                let mut request =
                    error_bot.send_message(reply_chat_id, format_command_error(command, &error));
                request = request.parse_mode(ParseMode::Html);
                if let Some(thread_id) = reply_thread_id {
                    request = request.message_thread_id(thread_id);
                }
                let _ = request.await;
            }
        }
        respond(())
    })
    .await;
}

async fn handle_message(bot: Bot, msg: Message) -> Result<(), String> {
    let Some(text) = msg.text().map(str::to_owned) else {
        return Ok(());
    };

    match parse_command(&text) {
        Some(BotCommand::Analyze) => handle_analyze_command(bot, msg, &text).await,
        Some(BotCommand::Voice) => handle_voice_command(bot, msg, &text).await,
        None => Ok(()),
    }
}

async fn handle_analyze_command(bot: Bot, msg: Message, text: &str) -> Result<(), String> {
    log_command(&msg, text, "analyze");

    let options = parse_analyze_options(text)?;
    let input = find_input_audio(&msg).ok_or_else(|| analyze_usage_hint().to_string())?;
    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        label = %input.label,
        "selected input audio"
    );

    let mut progress_request = bot
        .send_message(
            msg.chat.id,
            format!(
                "<b>Received</b> {}.\nDownloading and analyzing it now. This can take a few seconds.",
                escape_html(&input.label)
            ),
        )
        .parse_mode(ParseMode::Html);
    if let Some(thread_id) = msg.thread_id {
        progress_request = progress_request.message_thread_id(thread_id);
    }
    progress_request
        .await
        .map_err(|error| format!("failed to send progress message: {error}"))?;

    let bytes = download_input_audio(&bot, &msg, &input).await?;

    let file_name = input.file_name.clone();
    let (report_text, graphs) = task::spawn_blocking(move || {
        let rendered = analyze_audio_bytes(&bytes, file_name.as_deref(), &input.label, &options)?;
        Ok::<_, String>((rendered.report_text, rendered.graphs))
    })
    .await
    .map_err(|error| format!("analysis task failed: {error}"))??;
    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        report_len = report_text.len(),
        graph_count = graphs.len(),
        "analysis completed"
    );

    send_long_message(&bot, msg.chat.id, msg.thread_id, &report_text).await?;
    send_graphs(&bot, msg.chat.id, msg.thread_id, graphs).await
}

async fn handle_voice_command(bot: Bot, msg: Message, text: &str) -> Result<(), String> {
    log_command(&msg, text, "voice");

    let input = find_input_audio(&msg).ok_or_else(|| voice_usage_hint().to_string())?;
    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        label = %input.label,
        "selected input audio"
    );

    let mut progress_request = bot
        .send_message(
            msg.chat.id,
            format!(
                "<b>Received</b> {}.\nDownloading and preparing it as a voice message now. This can take a few seconds.",
                escape_html(&input.label)
            ),
        )
        .parse_mode(ParseMode::Html);
    if let Some(thread_id) = msg.thread_id {
        progress_request = progress_request.message_thread_id(thread_id);
    }
    progress_request
        .await
        .map_err(|error| format!("failed to send progress message: {error}"))?;

    let bytes = download_input_audio(&bot, &msg, &input).await?;

    let duration_seconds = input.duration_seconds;
    let file_name = input.file_name.clone();
    let is_voice_message = input.is_voice_message;
    let voice_bytes = task::spawn_blocking(move || {
        prepare_voice_upload(&bytes, file_name.as_deref(), is_voice_message)
    })
    .await
    .map_err(|error| format!("voice conversion task failed: {error}"))??;
    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        bytes = voice_bytes.len(),
        "prepared voice upload"
    );

    let mut request = bot.send_voice(
        msg.chat.id,
        InputFile::memory(voice_bytes).file_name("voice.ogg"),
    );
    if let Some(duration_seconds) = duration_seconds {
        request = request.duration(duration_seconds);
    }
    if let Some(thread_id) = msg.thread_id {
        request = request.message_thread_id(thread_id);
    }
    request
        .await
        .map_err(|error| format!("failed to send voice message: {error}"))?;

    Ok(())
}

async fn download_input_audio(
    bot: &Bot,
    msg: &Message,
    input: &InputAudio,
) -> Result<Vec<u8>, String> {
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

    Ok(bytes)
}

async fn send_long_message(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<ThreadId>,
    text: &str,
) -> Result<(), String> {
    const LIMIT: usize = 3500;
    if text.len() <= LIMIT {
        let mut request = bot
            .send_message(chat_id, text.to_string())
            .parse_mode(ParseMode::Html);
        if let Some(thread_id) = thread_id {
            request = request.message_thread_id(thread_id);
        }
        request
            .await
            .map_err(|error| format!("failed to send analysis result: {error}"))?;
        return Ok(());
    }

    let mut current = String::new();
    for line in text.lines() {
        if !current.is_empty() && current.len() + line.len() + 1 > LIMIT {
            let mut request = bot
                .send_message(chat_id, current.clone())
                .parse_mode(ParseMode::Html);
            if let Some(thread_id) = thread_id {
                request = request.message_thread_id(thread_id);
            }
            request
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
        let mut request = bot
            .send_message(chat_id, current)
            .parse_mode(ParseMode::Html);
        if let Some(thread_id) = thread_id {
            request = request.message_thread_id(thread_id);
        }
        request
            .await
            .map_err(|error| format!("failed to send final analysis chunk: {error}"))?;
    }

    Ok(())
}

async fn send_graphs(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<ThreadId>,
    graphs: Vec<GraphImage>,
) -> Result<(), String> {
    if graphs.is_empty() {
        return Ok(());
    }

    let title_list = graphs
        .iter()
        .map(|graph| graph.title.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let media = graphs
        .into_iter()
        .enumerate()
        .map(|(index, graph)| {
            let photo =
                InputMediaPhoto::new(InputFile::memory(graph.png_bytes).file_name(graph.file_name));
            if index == 0 {
                InputMedia::Photo(
                    photo
                        .caption(format!(
                            "<b>Analysis graphs</b>\n{}",
                            escape_html(&title_list)
                        ))
                        .parse_mode(ParseMode::Html),
                )
            } else {
                InputMedia::Photo(photo)
            }
        })
        .collect::<Vec<_>>();

    let mut request = bot.send_media_group(chat_id, media);
    if let Some(thread_id) = thread_id {
        request = request.message_thread_id(thread_id);
    }
    request
        .await
        .map_err(|error| format!("failed to send graph group: {error}"))?;

    Ok(())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn parse_command(text: &str) -> Option<BotCommand> {
    let Some(command) = text.split_whitespace().next() else {
        return None;
    };

    let command_name = command.split('@').next().unwrap_or(command);
    match command_name {
        "/analyze" => Some(BotCommand::Analyze),
        "/voice" => Some(BotCommand::Voice),
        _ => None,
    }
}

fn format_command_error(command: BotCommand, error: &str) -> String {
    let title = match command {
        BotCommand::Analyze => "Could not analyze that audio.",
        BotCommand::Voice => "Could not re-upload that audio as a voice message.",
    };

    format!("<b>{title}</b>\n\n{}", escape_html(error))
}

fn voice_usage_hint() -> &'static str {
    "Reply to a voice message or audio file with <code>/voice</code>."
}

fn log_command(msg: &Message, text: &str, command_name: &str) {
    info!(
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        command_name,
        command_text = text,
        "received bot command"
    );
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

#[cfg(test)]
mod tests {
    use super::{BotCommand, parse_command};

    #[test]
    fn parses_plain_commands() {
        assert_eq!(parse_command("/analyze"), Some(BotCommand::Analyze));
        assert_eq!(parse_command("/voice"), Some(BotCommand::Voice));
    }

    #[test]
    fn parses_commands_with_bot_mentions_and_arguments() {
        assert_eq!(
            parse_command("/analyze@voicebot +graph"),
            Some(BotCommand::Analyze)
        );
        assert_eq!(
            parse_command("/voice@voicebot now"),
            Some(BotCommand::Voice)
        );
    }

    #[test]
    fn ignores_unknown_commands() {
        assert_eq!(parse_command("/start"), None);
        assert_eq!(parse_command("hello"), None);
    }
}
