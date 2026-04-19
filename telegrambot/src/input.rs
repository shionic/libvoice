use teloxide::types::{Audio, Document, FileId, Message};

#[derive(Clone, Debug)]
pub struct InputAudio {
    pub file_id: FileId,
    pub file_name: Option<String>,
    pub label: String,
    pub duration_seconds: Option<u32>,
    pub is_voice_message: bool,
}

pub fn find_input_audio(msg: &Message) -> Option<InputAudio> {
    extract_audio_from_message(msg)
        .or_else(|| msg.reply_to_message().and_then(extract_audio_from_message))
}

fn extract_audio_from_message(msg: &Message) -> Option<InputAudio> {
    if let Some(voice) = msg.voice() {
        return Some(InputAudio {
            file_id: voice.file.id.clone(),
            file_name: Some("voice.ogg".to_string()),
            label: "voice message".to_string(),
            duration_seconds: Some(voice.duration.seconds()),
            is_voice_message: true,
        });
    }

    if let Some(audio) = msg.audio() {
        if !is_supported_audio_file(audio) {
            return None;
        }
        return Some(InputAudio {
            file_id: audio.file.id.clone(),
            file_name: audio.file_name.clone(),
            label: audio
                .file_name
                .clone()
                .unwrap_or_else(|| "audio file".to_string()),
            duration_seconds: Some(audio.duration.seconds()),
            is_voice_message: false,
        });
    }

    let document = msg.document()?;
    if !is_supported_audio_document(document) {
        return None;
    }

    Some(InputAudio {
        file_id: document.file.id.clone(),
        file_name: document.file_name.clone(),
        label: document
            .file_name
            .clone()
            .unwrap_or_else(|| "audio file".to_string()),
        duration_seconds: None,
        is_voice_message: false,
    })
}

fn is_supported_audio_document(document: &Document) -> bool {
    is_audio_name_or_mime(document.file_name.as_deref(), document.mime_type.as_ref())
}

fn is_supported_audio_file(audio: &Audio) -> bool {
    is_audio_name_or_mime(audio.file_name.as_deref(), audio.mime_type.as_ref())
}

fn is_audio_name_or_mime(file_name: Option<&str>, mime_type: Option<&mime::Mime>) -> bool {
    let file_name_ok = file_name
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            [".ogg", ".oga", ".opus", ".wav", ".mp3", ".m4a", ".flac"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
        })
        .unwrap_or(false);

    let mime_ok = mime_type
        .map(|mime| mime.type_() == mime::AUDIO || mime.as_ref() == "application/ogg")
        .unwrap_or(false);

    file_name_ok || mime_ok
}
