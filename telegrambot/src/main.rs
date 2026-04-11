mod audio;
mod graphs;
mod input;
mod options;
mod report;

use audio::{analyze_samples, audio_duration_seconds, clip_audio_seconds, decode_audio_bytes};
use graphs::{GraphImage, build_spectrum_graph, generate_graphs};
use input::find_input_audio;
use options::{ResolvedClip, analyze_usage_hint, parse_analyze_options};
use report::format_report;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InputFile, InputMedia, InputMediaPhoto, Message, ParseMode, ThreadId};
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
        let reply_thread_id = msg.thread_id;
        let command_text = msg.text().map(str::to_owned);
        let error_bot = bot.clone();
        if let Err(error) = handle_message(bot, msg).await {
            error!(%error, "request handling failed");
            if command_text.as_deref().is_some_and(is_analyze_command) {
                let mut request = error_bot.send_message(
                    reply_chat_id,
                    format!(
                        "<b>Could not analyze that audio.</b>\n\n{}",
                        escape_html(&error)
                    ),
                );
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
    let Some(text) = msg.text() else {
        return Ok(());
    };
    if !is_analyze_command(text) {
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
    let (report_text, graphs) = task::spawn_blocking(move || {
        let decoded = decode_audio_bytes(&bytes, file_name.as_deref())?;
        let resolved_clip = options
            .clip
            .as_ref()
            .map(|clip| clip.resolve(audio_duration_seconds(&decoded)))
            .transpose()?;
        let analysis_audio = match resolved_clip.as_ref() {
            Some(clip) => clip_audio_seconds(&decoded, clip.from_seconds, clip.to_seconds)?,
            None => decoded,
        };
        let report = analyze_samples(&analysis_audio, options.spectrum);
        let report_label = format_report_label(&input.label, resolved_clip.as_ref());
        let report_text = format_report(&report_label, &analysis_audio, &report, &options);
        let mut graphs = if options.graph {
            generate_graphs(&report)?
        } else {
            Vec::new()
        };
        if options.spectrum {
            if let Some(graph) = build_spectrum_graph(&report)? {
                graphs.push(graph);
            }
        }
        Ok::<_, String>((report_text, graphs))
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

fn format_report_label(label: &str, clip: Option<&ResolvedClip>) -> String {
    match clip {
        Some(clip) => format!(
            "{} [{} .. {}]",
            label,
            format_time_seconds(clip.from_seconds),
            format_time_seconds(clip.to_seconds)
        ),
        None => label.to_string(),
    }
}

fn format_time_seconds(seconds: f32) -> String {
    let total_millis = (seconds * 1000.0).round().max(0.0) as u64;
    let total_seconds = total_millis / 1000;
    let minutes = total_seconds / 60;
    let secs = total_seconds % 60;
    let millis = total_millis % 1000;

    if minutes > 0 {
        if millis == 0 {
            format!("{minutes}m{secs:02}s")
        } else {
            format!("{minutes}m{secs:02}.{millis:03}s")
        }
    } else if millis == 0 {
        format!("{secs}s")
    } else {
        format!("{secs}.{millis:03}s")
    }
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

fn is_analyze_command(text: &str) -> bool {
    let Some(command) = text.split_whitespace().next() else {
        return false;
    };

    let command_name = command.split('@').next().unwrap_or(command);
    command_name == "/analyze"
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
