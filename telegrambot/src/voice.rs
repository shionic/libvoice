use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

use tempfile::Builder;

pub fn prepare_voice_upload(
    bytes: &[u8],
    file_name: Option<&str>,
    already_voice_message: bool,
) -> Result<Vec<u8>, String> {
    if already_voice_message {
        return Ok(bytes.to_vec());
    }

    transcode_audio_bytes_to_voice(bytes, file_name)
}

fn transcode_audio_bytes_to_voice(
    bytes: &[u8],
    file_name: Option<&str>,
) -> Result<Vec<u8>, String> {
    let mut temp_input = Builder::new()
        .suffix(&input_suffix(file_name))
        .tempfile()
        .map_err(|error| format!("failed to create temp input file: {error}"))?;

    temp_input
        .write_all(bytes)
        .map_err(|error| format!("failed to write temp audio file: {error}"))?;
    temp_input
        .flush()
        .map_err(|error| format!("failed to flush temp audio file: {error}"))?;

    let output = std::process::Command::new("ffmpeg")
        .arg("-nostdin")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(temp_input.path())
        .arg("-map")
        .arg("0:a:0")
        .arg("-vn")
        .arg("-c:a")
        .arg("libopus")
        .arg("-b:a")
        .arg("48k")
        .arg("-ac")
        .arg("1")
        .arg("-f")
        .arg("ogg")
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("failed to wait for ffmpeg: {error}"))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("ffmpeg exited with {}", output.status)
        } else {
            detail
        });
    }

    if output.stdout.is_empty() {
        return Err("ffmpeg produced an empty voice file".to_string());
    }

    Ok(output.stdout)
}

fn input_suffix(file_name: Option<&str>) -> String {
    file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::input_suffix;

    #[test]
    fn keeps_original_extension_for_temp_input() {
        assert_eq!(input_suffix(Some("sample.m4a")), ".m4a");
    }

    #[test]
    fn omits_suffix_when_no_extension_is_present() {
        assert_eq!(input_suffix(Some("sample")), "");
        assert_eq!(input_suffix(None), "");
    }
}
