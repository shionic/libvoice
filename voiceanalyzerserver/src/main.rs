use async_stream::stream;
use axum::body::{Body, to_bytes};
use axum::extract::{DefaultBodyLimit, Query, Request};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use clap::Parser;
use futures_util::StreamExt;
use libvoice::{
    AnalysisReport, AnalyzerConfig, ChunkAnalysis, FrameAnalysis, OverallAnalysis, VoiceAnalyzer,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::net::SocketAddr;
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const MAX_UPLOAD_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PcmEncoding {
    Auto,
    F32Le,
    S16Le,
}

#[derive(Debug, Deserialize)]
struct AnalyzeQuery {
    #[serde(default)]
    include_frames: bool,
    #[serde(default)]
    pcm_encoding: Option<PcmEncoding>,
    sample_rate: Option<u32>,
    #[serde(default)]
    channels: Option<usize>,
    frame_size: Option<usize>,
    hop_size: Option<usize>,
    min_pitch_hz: Option<f32>,
    max_pitch_hz: Option<f32>,
    pitch_clarity_threshold: Option<f32>,
    rolloff_ratio: Option<f32>,
    voiced_rms_threshold: Option<f32>,
    voiced_max_spectral_flatness: Option<f32>,
    voiced_max_zero_crossing_rate: Option<f32>,
}

#[derive(Debug, Serialize)]
struct AnalyzeResponse {
    backend: String,
    sample_rate: u32,
    channels: usize,
    duration_seconds: f32,
    report: AnalysisReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames: Option<Vec<FrameAnalysis>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamEvent {
    Started {
        backend: &'static str,
        sample_rate: u32,
        channels: usize,
        config: AnalyzerConfig,
    },
    Frame {
        frame: FrameAnalysis,
    },
    Chunk {
        chunk: ChunkAnalysis,
    },
    SummaryPartial {
        processed_seconds: f32,
        overall: OverallAnalysis,
    },
    Summary {
        processed_seconds: f32,
        overall: OverallAnalysis,
    },
    Error {
        message: String,
    },
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

#[derive(Debug)]
struct DecodedAudio {
    backend: &'static str,
    sample_rate: u32,
    channels: usize,
    samples: Vec<f32>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let app = Router::new()
        .route("/", get(health_handler))
        .route("/v1/analyze", post(analyze_handler))
        .route("/v1/analyze/stream", post(stream_handler))
        .layer(DefaultBodyLimit::disable());

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .expect("binding server listener must succeed");

    println!("voiceanalyzerserver listening on http://{}", args.bind);

    axum::serve(listener, app)
        .await
        .expect("server must run until shutdown");
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "service": "voiceanalyzerserver",
        "status": "ok",
        "routes": {
            "analyze": "POST /v1/analyze",
            "stream": "POST /v1/analyze/stream"
        }
    }))
}

async fn analyze_handler(
    Query(query): Query<AnalyzeQuery>,
    request: Request,
) -> Result<Json<AnalyzeResponse>, ApiError> {
    let body = to_bytes(request.into_body(), MAX_UPLOAD_BYTES)
        .await
        .map_err(|error| bad_request(format!("failed to read request body: {error}")))?;

    if body.is_empty() {
        return Err(bad_request("request body was empty"));
    }

    let decoded = decode_one_shot_audio(body.as_ref(), &query)?;
    let config = build_config(decoded.sample_rate, &query)?;
    let duration_seconds = decoded.samples.len() as f32 / decoded.sample_rate as f32;

    let (report, frames) = if query.include_frames {
        let mut analyzer = VoiceAnalyzer::new(config.clone());
        let (chunk, frames) = analyzer.process_chunk_with_frames(&decoded.samples);
        let report = AnalysisReport {
            config,
            chunks: vec![chunk],
            overall: analyzer.finalize(),
        };
        (report, Some(frames))
    } else {
        (
            VoiceAnalyzer::analyze_buffer(config, &decoded.samples),
            None,
        )
    };

    Ok(Json(AnalyzeResponse {
        backend: decoded.backend.to_string(),
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        duration_seconds,
        report,
        frames,
    }))
}

async fn stream_handler(
    Query(query): Query<AnalyzeQuery>,
    request: Request,
) -> Result<Response, ApiError> {
    let encoding = query.pcm_encoding.unwrap_or(PcmEncoding::Auto);
    if encoding == PcmEncoding::Auto {
        return Err(bad_request(
            "streaming requires `pcm_encoding=f32_le` or `pcm_encoding=s16_le`",
        ));
    }

    let sample_rate = query
        .sample_rate
        .ok_or_else(|| bad_request("streaming requires `sample_rate`"))?;
    if sample_rate == 0 {
        return Err(bad_request("`sample_rate` must be greater than 0"));
    }

    let channels = query.channels.unwrap_or(1);
    if channels == 0 {
        return Err(bad_request("`channels` must be greater than 0"));
    }

    let config = build_config(sample_rate, &query)?;
    let mut analyzer = VoiceAnalyzer::new(config.clone());
    let mut body_stream = request.into_body().into_data_stream();

    let output = stream! {
        let mut streamed_samples = 0usize;
        let mut all_frames = Vec::new();

        yield ok_line(&StreamEvent::Started {
            backend: "raw_pcm_stream",
            sample_rate,
            channels,
            config,
        });

        let mut pending = Vec::new();
        while let Some(next) = body_stream.next().await {
            let bytes = match next {
                Ok(bytes) => bytes,
                Err(error) => {
                    yield ok_line(&StreamEvent::Error {
                        message: format!("failed to read request stream: {error}"),
                    });
                    return;
                }
            };

            pending.extend_from_slice(&bytes);
            let samples = match drain_pcm_samples(&mut pending, encoding, channels) {
                Ok(samples) => samples,
                Err(error) => {
                    yield ok_line(&StreamEvent::Error {
                        message: error,
                    });
                    return;
                }
            };

            if samples.is_empty() {
                continue;
            }

            streamed_samples += samples.len();
            let (chunk, frames) = analyzer.process_chunk_with_frames(&samples);
            all_frames.extend(frames.iter().cloned());
            for frame in frames {
                yield ok_line(&StreamEvent::Frame { frame });
            }

            yield ok_line(&StreamEvent::Chunk { chunk });
            yield ok_line(&StreamEvent::SummaryPartial {
                processed_seconds: streamed_samples as f32 / sample_rate as f32,
                overall: summarize_partial_overall(streamed_samples, &all_frames),
            });
        }

        if !pending.is_empty() {
            yield ok_line(&StreamEvent::Error {
                message: "request body ended with a partial PCM sample".to_string(),
            });
            return;
        }

        let overall = analyzer.finalize();
        yield ok_line(&StreamEvent::Summary {
            processed_seconds: overall.processed_samples as f32 / sample_rate as f32,
            overall,
        });
    };

    let mut response = Response::new(Body::from_stream(output));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    Ok(response)
}

fn build_config(sample_rate: u32, query: &AnalyzeQuery) -> Result<AnalyzerConfig, ApiError> {
    let mut config = AnalyzerConfig::new(sample_rate);

    if let Some(frame_size) = query.frame_size {
        if frame_size < 8 {
            return Err(bad_request("`frame_size` must be at least 8"));
        }
        config.frame_size = frame_size;
    }
    if let Some(hop_size) = query.hop_size {
        if hop_size == 0 {
            return Err(bad_request("`hop_size` must be greater than 0"));
        }
        config.hop_size = hop_size;
    }
    if let Some(min_pitch_hz) = query.min_pitch_hz {
        config.min_pitch_hz = min_pitch_hz;
    }
    if let Some(max_pitch_hz) = query.max_pitch_hz {
        config.max_pitch_hz = max_pitch_hz;
    }
    if let Some(pitch_clarity_threshold) = query.pitch_clarity_threshold {
        config.pitch_clarity_threshold = pitch_clarity_threshold;
    }
    if let Some(rolloff_ratio) = query.rolloff_ratio {
        config.rolloff_ratio = rolloff_ratio;
    }
    if let Some(voiced_rms_threshold) = query.voiced_rms_threshold {
        config.voiced_rms_threshold = voiced_rms_threshold;
    }
    if let Some(voiced_max_spectral_flatness) = query.voiced_max_spectral_flatness {
        config.voiced_max_spectral_flatness = voiced_max_spectral_flatness;
    }
    if let Some(voiced_max_zero_crossing_rate) = query.voiced_max_zero_crossing_rate {
        config.voiced_max_zero_crossing_rate = voiced_max_zero_crossing_rate;
    }

    if config.min_pitch_hz <= 0.0 || config.max_pitch_hz <= 0.0 {
        return Err(bad_request("pitch bounds must be greater than 0"));
    }
    if config.min_pitch_hz >= config.max_pitch_hz {
        return Err(bad_request(
            "`min_pitch_hz` must be smaller than `max_pitch_hz`",
        ));
    }
    if config.hop_size > config.frame_size {
        return Err(bad_request(
            "`hop_size` must be less than or equal to `frame_size`",
        ));
    }

    Ok(config)
}

fn decode_one_shot_audio(bytes: &[u8], query: &AnalyzeQuery) -> Result<DecodedAudio, ApiError> {
    match query.pcm_encoding.unwrap_or(PcmEncoding::Auto) {
        PcmEncoding::Auto => decode_with_symphonia(bytes),
        encoding => decode_pcm_bytes(
            bytes,
            encoding,
            query
                .sample_rate
                .ok_or_else(|| bad_request("PCM input requires `sample_rate`"))?,
            query.channels.unwrap_or(1),
        ),
    }
}

fn decode_with_symphonia(bytes: &[u8]) -> Result<DecodedAudio, ApiError> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| unsupported_media(format!("failed to probe audio stream: {error}")))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| unsupported_media("no default audio track found"))?;
    let codec_params = track.codec_params.clone();
    if codec_params.codec == CODEC_TYPE_NULL {
        return Err(unsupported_media("unsupported or missing codec parameters"));
    }

    let sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| unsupported_media("missing sample rate in audio stream"))?;
    let channels = codec_params
        .channels
        .map(|layout| layout.count())
        .ok_or_else(|| unsupported_media("missing channel layout in audio stream"))?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|error| unsupported_media(format!("failed to create decoder: {error}")))?;

    let mut mono_scratch = Vec::new();
    let mut samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err(unsupported_media(
                    "decoder reset required; chained streams are not supported",
                ));
            }
            Err(error) => {
                return Err(unsupported_media(format!(
                    "failed while reading packets: {error}"
                )));
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(unsupported_media(format!("decode failed: {error}"))),
        };

        let mono = fold_to_mono(decoded, channels, &mut mono_scratch);
        samples.extend_from_slice(mono);
    }

    Ok(DecodedAudio {
        backend: "symphonia",
        sample_rate,
        channels,
        samples,
    })
}

fn decode_pcm_bytes(
    bytes: &[u8],
    encoding: PcmEncoding,
    sample_rate: u32,
    channels: usize,
) -> Result<DecodedAudio, ApiError> {
    if sample_rate == 0 {
        return Err(bad_request("`sample_rate` must be greater than 0"));
    }
    if channels == 0 {
        return Err(bad_request("`channels` must be greater than 0"));
    }

    let mut pending = bytes.to_vec();
    let samples = drain_pcm_samples(&mut pending, encoding, channels).map_err(bad_request)?;
    if !pending.is_empty() {
        return Err(bad_request("request body ended with a partial PCM sample"));
    }

    Ok(DecodedAudio {
        backend: match encoding {
            PcmEncoding::F32Le => "pcm_f32le",
            PcmEncoding::S16Le => "pcm_s16le",
            PcmEncoding::Auto => "auto",
        },
        sample_rate,
        channels,
        samples,
    })
}

fn drain_pcm_samples(
    pending: &mut Vec<u8>,
    encoding: PcmEncoding,
    channels: usize,
) -> Result<Vec<f32>, String> {
    let bytes_per_channel = match encoding {
        PcmEncoding::F32Le => std::mem::size_of::<f32>(),
        PcmEncoding::S16Le => std::mem::size_of::<i16>(),
        PcmEncoding::Auto => {
            return Err("PCM decoding requires an explicit encoding".to_string());
        }
    };

    let frame_bytes = bytes_per_channel
        .checked_mul(channels)
        .ok_or_else(|| "PCM channel count was too large".to_string())?;
    if frame_bytes == 0 {
        return Err("PCM frame size was zero".to_string());
    }

    let complete_bytes = pending.len() / frame_bytes * frame_bytes;
    let mut samples = Vec::with_capacity(complete_bytes / frame_bytes);

    for frame in pending[..complete_bytes].chunks_exact(frame_bytes) {
        let mut sum = 0.0_f32;
        for channel in 0..channels {
            let offset = channel * bytes_per_channel;
            let sample = match encoding {
                PcmEncoding::F32Le => f32::from_le_bytes([
                    frame[offset],
                    frame[offset + 1],
                    frame[offset + 2],
                    frame[offset + 3],
                ]),
                PcmEncoding::S16Le => {
                    let sample = i16::from_le_bytes([frame[offset], frame[offset + 1]]);
                    sample as f32 / i16::MAX as f32
                }
                PcmEncoding::Auto => unreachable!(),
            };
            sum += sample;
        }
        samples.push(sum / channels as f32);
    }

    if complete_bytes > 0 {
        pending.drain(..complete_bytes);
    }

    Ok(samples)
}

fn fold_to_mono<'a>(
    decoded: AudioBufferRef<'_>,
    channel_count: usize,
    mono: &'a mut Vec<f32>,
) -> &'a [f32] {
    let channels = channel_count.max(1);
    match decoded {
        AudioBufferRef::F32(buffer) => {
            average_channels_into(mono, buffer.chan(0).len(), channels, |index, channel| {
                *buffer.chan(channel).get(index).unwrap_or(&0.0)
            });
            mono.as_slice()
        }
        other => {
            let mut sample_buffer =
                SampleBuffer::<f32>::new(other.capacity() as u64, *other.spec());
            sample_buffer.copy_interleaved_ref(other);
            let samples = sample_buffer.samples();
            average_interleaved_channels_into(mono, samples, channels);
            mono.as_slice()
        }
    }
}

fn average_channels_into<F>(
    mono: &mut Vec<f32>,
    frames: usize,
    channel_count: usize,
    mut sample_at: F,
) where
    F: FnMut(usize, usize) -> f32,
{
    mono.clear();
    mono.reserve(frames.saturating_sub(mono.capacity()));
    for index in 0..frames {
        let mut sum = 0.0_f32;
        for channel in 0..channel_count {
            sum += sample_at(index, channel);
        }
        mono.push(sum / channel_count as f32);
    }
}

fn average_interleaved_channels_into(mono: &mut Vec<f32>, samples: &[f32], channel_count: usize) {
    mono.clear();
    mono.reserve(
        samples
            .len()
            .saturating_div(channel_count)
            .saturating_sub(mono.capacity()),
    );

    for frame in samples.chunks_exact(channel_count) {
        let mut sum = 0.0_f32;
        for &sample in frame {
            sum += sample;
        }
        mono.push(sum / channel_count as f32);
    }
}

fn event_line(event: &StreamEvent) -> Bytes {
    let mut encoded = serde_json::to_vec(event).expect("stream event must serialize");
    encoded.push(b'\n');
    Bytes::from(encoded)
}

fn ok_line(event: &StreamEvent) -> Result<Bytes, Infallible> {
    Ok(event_line(event))
}

fn summarize_partial_overall(
    processed_samples: usize,
    frames: &[FrameAnalysis],
) -> OverallAnalysis {
    OverallAnalysis {
        processed_samples,
        frame_count: frames.len(),
        pitch_hz: summarize_optional_stats(frames.iter().filter_map(|frame| frame.pitch_hz)),
        formants: summarize_formants(frames),
        spectral: summarize_spectral(frames),
        energy: summarize_required_stats(frames.iter().map(|frame| frame.energy)),
        jitter: None,
    }
}

fn summarize_formants(frames: &[FrameAnalysis]) -> Option<libvoice::FormantSummary> {
    let f1_hz = summarize_optional_stats(frames.iter().filter_map(|frame| frame.formant_1_hz))?;
    let f2_hz = summarize_optional_stats(frames.iter().filter_map(|frame| frame.formant_2_hz))?;
    let f3_hz = summarize_optional_stats(frames.iter().filter_map(|frame| frame.formant_3_hz))?;

    Some(libvoice::FormantSummary {
        f1_hz,
        f2_hz,
        f3_hz,
    })
}

fn summarize_spectral(frames: &[FrameAnalysis]) -> Option<libvoice::SpectralSummary> {
    if frames.is_empty() {
        return None;
    }

    Some(libvoice::SpectralSummary {
        rolloff_hz: summarize_required_stats(frames.iter().map(|frame| frame.spectral_rolloff_hz))
            .expect("non-empty frames must produce rolloff stats"),
        centroid_hz: summarize_required_stats(
            frames.iter().map(|frame| frame.spectral_centroid_hz),
        )
        .expect("non-empty frames must produce centroid stats"),
        bandwidth_hz: summarize_required_stats(
            frames.iter().map(|frame| frame.spectral_bandwidth_hz),
        )
        .expect("non-empty frames must produce bandwidth stats"),
        flatness: summarize_required_stats(frames.iter().map(|frame| frame.spectral_flatness))
            .expect("non-empty frames must produce flatness stats"),
        zcr: summarize_required_stats(frames.iter().map(|frame| frame.zcr))
            .expect("non-empty frames must produce zcr stats"),
        rms: summarize_required_stats(frames.iter().map(|frame| frame.rms))
            .expect("non-empty frames must produce rms stats"),
        hnr_db: summarize_required_stats(frames.iter().map(|frame| frame.hnr_db))
            .expect("non-empty frames must produce hnr stats"),
    })
}

fn summarize_optional_stats<I>(values: I) -> Option<libvoice::SummaryStats>
where
    I: Iterator<Item = f32>,
{
    summarize_values(values.filter(|value| value.is_finite()).collect())
}

fn summarize_required_stats<I>(values: I) -> Option<libvoice::SummaryStats>
where
    I: Iterator<Item = f32>,
{
    summarize_values(values.filter(|value| value.is_finite()).collect())
}

fn summarize_values(mut values: Vec<f32>) -> Option<libvoice::SummaryStats> {
    if values.is_empty() {
        return None;
    }

    values.sort_by(|a, b| a.total_cmp(b));
    let count = values.len();
    let mean = values.iter().sum::<f32>() / count as f32;
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f32>()
        / count as f32;

    Some(libvoice::SummaryStats {
        count,
        mean,
        std: variance.sqrt(),
        median: percentile_sorted(&values, 0.5),
        min: values[0],
        max: values[count - 1],
        p5: percentile_sorted(&values, 0.05),
        p95: percentile_sorted(&values, 0.95),
    })
}

fn percentile_sorted(values: &[f32], percentile: f32) -> f32 {
    if values.len() == 1 {
        return values[0];
    }

    let position = percentile.clamp(0.0, 1.0) * (values.len() - 1) as f32;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        return values[lower];
    }

    let weight = position - lower as f32;
    values[lower] * (1.0 - weight) + values[upper] * weight
}

fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: message.into(),
    }
}

fn unsupported_media(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
        message: message.into(),
    }
}

impl Display for ApiError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}
