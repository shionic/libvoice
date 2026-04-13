# voiceanalyzerserver API

`voiceanalyzerserver` exposes `libvoice` over HTTP.

Base routes:

- `GET /`
- `POST /v1/analyze`
- `POST /v1/analyze/stream`
- `GET /v2/analyze/ws`

The server listens on `127.0.0.1:3000` by default. Override with:

```bash
cargo run --release --manifest-path voiceanalyzerserver/Cargo.toml -- --bind 0.0.0.0:3000
```

## Overview

`POST /v1/analyze`

- One-shot request/response.
- Accepts normal audio file bytes with automatic decoding through Symphonia.
- Also accepts raw PCM if `pcm_encoding` and `sample_rate` are provided.
- Returns JSON.

`POST /v1/analyze/stream`

- Incremental streaming analysis.
- Accepts raw PCM only.
- Returns NDJSON (`application/x-ndjson`).
- Emits per-frame events, per-chunk summaries, rolling partial overall summaries, and a final summary.

`GET /v2/analyze/ws`

- Incremental streaming analysis over WebSocket.
- Accepts client binary MessagePack messages carrying raw PCM chunks.
- Returns server binary MessagePack events.
- Emits per-batch frame events, per-chunk summaries, rolling partial overall summaries, and a final summary.

## Query Parameters

Both endpoints accept these analyzer parameters:

- `frame_size`
- `hop_size`
- `min_pitch_hz`
- `max_pitch_hz`
- `pitch_clarity_threshold`
- `rolloff_ratio`
- `voiced_rms_threshold`
- `voiced_max_spectral_flatness`
- `voiced_max_zero_crossing_rate`

Shared audio input parameters:

- `pcm_encoding`
  - `auto`
  - `f32_le`
  - `s16_le`
- `sample_rate`
- `channels`

Extra parameter for one-shot analysis:

- `include_frames=true|false`

Notes:

- `pcm_encoding=auto` is the default for `/v1/analyze`.
- `/v1/analyze/stream` requires `pcm_encoding=f32_le` or `pcm_encoding=s16_le`.
- `/v1/analyze/stream` requires `sample_rate`.
- `/v2/analyze/ws` requires `pcm_encoding=f32_le` or `pcm_encoding=s16_le`.
- `/v2/analyze/ws` requires `sample_rate`.
- `channels` defaults to `1`.
- The server folds multichannel PCM to mono by averaging channels.

## Health

Request:

```bash
curl http://127.0.0.1:3000/
```

Response:

```json
{
  "service": "voiceanalyzerserver",
  "status": "ok",
  "routes": {
    "analyze": "POST /v1/analyze",
    "stream": "POST /v1/analyze/stream"
  }
}
```

## One-Shot Analysis

### Analyze an audio file

```bash
curl \
  -X POST \
  "http://127.0.0.1:3000/v1/analyze?include_frames=true" \
  -H "Content-Type: audio/wav" \
  --data-binary @sample.wav
```

### Analyze raw PCM

```bash
curl \
  -X POST \
  "http://127.0.0.1:3000/v1/analyze?pcm_encoding=f32_le&sample_rate=16000&channels=1" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @sample.f32
```

### Response shape

```json
{
  "backend": "symphonia",
  "sample_rate": 16000,
  "channels": 1,
  "duration_seconds": 1.248,
  "report": {
    "config": {
      "sample_rate": 16000,
      "frame_size": 2048,
      "hop_size": 512,
      "min_pitch_hz": 60.0,
      "max_pitch_hz": 500.0,
      "pitch_clarity_threshold": 0.6,
      "rolloff_ratio": 0.85,
      "voiced_rms_threshold": 0.015,
      "voiced_max_spectral_flatness": 0.45,
      "voiced_max_zero_crossing_rate": 0.25
    },
    "chunks": [
      {
        "chunk_index": 0,
        "input_samples": 19968,
        "frame_count": 36,
        "pitch_hz": { "...": "SummaryStats" },
        "spectral": { "...": "SpectralSummary" },
        "energy": { "...": "SummaryStats" },
        "jitter": null
      }
    ],
    "overall": {
      "processed_samples": 19968,
      "frame_count": 36,
      "pitch_hz": { "...": "SummaryStats" },
      "spectral": { "...": "SpectralSummary" },
      "energy": { "...": "SummaryStats" },
      "jitter": null
    },
    "frames": [
      {
        "frame_index": 0,
        "start_sample": 0,
        "start_seconds": 0.0,
        "end_sample": 2048,
        "end_seconds": 0.128,
        "pitch_hz": 219.6,
        "pitch_clarity": 0.92,
        "spectral_rolloff_hz": 312.5,
        "spectral_centroid_hz": 236.1,
        "spectral_bandwidth_hz": 88.4,
        "spectral_flatness": 0.01,
        "zcr": 0.03,
        "rms": 0.35,
        "hnr_db": 14.7,
        "energy": 0.12,
        "formants_hz": [],
        "formant_bandwidths_hz": [],
        "cumulative": {
          "processed_samples": 2048,
          "frame_count": 1,
          "pitch_hz": { "...": "SummaryStats" },
          "spectral": { "...": "SpectralSummary" },
          "formants": null,
          "energy": { "...": "SummaryStats" },
          "jitter": null
        }
      }
    ]
  },
  "frames": [
    {
      "frame_index": 0,
      "start_sample": 0,
      "start_seconds": 0.0,
      "end_sample": 2048,
      "end_seconds": 0.128,
      "pitch_hz": 219.6,
      "pitch_clarity": 0.92,
      "spectral_rolloff_hz": 312.5,
      "spectral_centroid_hz": 236.1,
      "spectral_bandwidth_hz": 88.4,
      "spectral_flatness": 0.01,
      "zcr": 0.03,
      "rms": 0.35,
      "hnr_db": 14.7,
      "energy": 0.12,
      "formants_hz": [],
      "formant_bandwidths_hz": [],
      "cumulative": {
        "processed_samples": 2048,
        "frame_count": 1,
        "pitch_hz": { "...": "SummaryStats" },
        "spectral": { "...": "SpectralSummary" },
        "formants": null,
        "energy": { "...": "SummaryStats" },
        "jitter": null
      }
    }
  ]
}
```

`report.frames` is always present. The top-level `frames` field is omitted unless
`include_frames=true`.

## Streaming Analysis

### Request contract

`POST /v1/analyze/stream` accepts raw PCM request bytes only.

Required query parameters:

- `pcm_encoding=f32_le` or `pcm_encoding=s16_le`
- `sample_rate`

Optional:

- `channels`
- all analyzer parameters listed above

Example:

```bash
curl \
  -N \
  -X POST \
  "http://127.0.0.1:3000/v1/analyze/stream?pcm_encoding=f32_le&sample_rate=16000&channels=1" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @sample.f32
```

`-N` disables client-side buffering so NDJSON events print as they arrive.

### Event stream

The response content type is:

```text
application/x-ndjson
```

Each line is a JSON object with a `type` field.

Event types:

- `started`
- `frame`
- `chunk`
- `summary_partial`
- `summary`
- `error`

Example NDJSON:

```json
{"type":"started","backend":"raw_pcm_stream","sample_rate":16000,"channels":1,"config":{"sample_rate":16000,"frame_size":2048,"hop_size":512,"min_pitch_hz":60.0,"max_pitch_hz":500.0,"pitch_clarity_threshold":0.6,"rolloff_ratio":0.85,"voiced_rms_threshold":0.015,"voiced_max_spectral_flatness":0.45,"voiced_max_zero_crossing_rate":0.25}}
{"type":"frame","frame":{"frame_index":0,"start_sample":0,"start_seconds":0.0,"end_sample":2048,"end_seconds":0.128,"pitch_hz":219.6,"pitch_clarity":0.92,"spectral_rolloff_hz":312.5,"spectral_centroid_hz":236.1,"spectral_bandwidth_hz":88.4,"spectral_flatness":0.01,"zcr":0.03,"rms":0.35,"hnr_db":14.7,"energy":0.12,"formants_hz":[],"formant_bandwidths_hz":[],"cumulative":{"processed_samples":2048,"frame_count":1,"pitch_hz":{"...":"SummaryStats"},"spectral":{"...":"SpectralSummary"},"formants":null,"energy":{"...":"SummaryStats"},"jitter":null}}}
{"type":"chunk","chunk":{"chunk_index":0,"input_samples":4096,"frame_count":5,"pitch_hz":{"count":5,"mean":220.1,"std":0.6,"median":220.0,"min":219.3,"max":221.0,"p5":219.4,"p95":220.9},"spectral":{"...":"SpectralSummary"},"energy":{"count":5,"mean":0.12,"std":0.001,"median":0.12,"min":0.119,"max":0.122,"p5":0.119,"p95":0.122},"jitter":null}}
{"type":"summary_partial","processed_seconds":0.256,"overall":{"processed_samples":4096,"frame_count":5,"pitch_hz":{"count":5,"mean":220.1,"std":0.6,"median":220.0,"min":219.3,"max":221.0,"p5":219.4,"p95":220.9},"spectral":{"...":"SpectralSummary"},"energy":{"count":5,"mean":0.12,"std":0.001,"median":0.12,"min":0.119,"max":0.122,"p5":0.119,"p95":0.122},"jitter":null}}
{"type":"summary","processed_seconds":1.248,"overall":{"processed_samples":19968,"frame_count":36,"pitch_hz":{"...":"SummaryStats"},"spectral":{"...":"SpectralSummary"},"energy":{"...":"SummaryStats"},"jitter":null}}
```

### Partial vs final summary

`summary_partial` is provisional.

- It reflects only frames received so far.
- It is calculated on the server, so the client does not need to maintain rolling aggregates.
- Percentiles and pitch summary values may shift as more frames arrive.

`summary` is the final end-of-stream result.

- It is emitted once, after the request body ends cleanly.
- It should be treated as the authoritative overall result.

## Error Handling

Errors are returned as JSON for one-shot requests:

```json
{
  "error": "request body was empty"
}
```

Streaming errors are emitted as NDJSON `error` events:

```json
{"type":"error","message":"request body ended with a partial PCM sample"}
```

Common error cases:

- missing `sample_rate` for PCM input
- invalid `pcm_encoding`
- partial PCM sample at end of stream
- unsupported compressed audio format on `/v1/analyze`
- invalid analyzer configuration such as `hop_size > frame_size`

## Transport Notes

Streaming over plain HTTP is acceptable when the network path supports full duplex behavior.

In practice, buffering by clients, reverse proxies, or load balancers can delay streamed events. For controlled deployments this NDJSON protocol is fine. For less predictable internet paths, a WebSocket transport may be more reliable later.

## WebSocket Protocol V2

`GET /v2/analyze/ws` upgrades to a WebSocket session.

The query string uses the same analyzer and PCM parameters as `/v1/analyze/stream`:

- `pcm_encoding=f32_le` or `pcm_encoding=s16_le`
- `sample_rate`
- optional `channels`
- optional analyzer parameters such as `frame_size`, `hop_size`, `min_pitch_hz`, `max_pitch_hz`

The server does not wait for application-level acknowledgements. Clients should send audio chunks continuously and read server events as they arrive.

### Wire format

- client to server: binary WebSocket messages containing MessagePack objects
- server to client: binary WebSocket messages containing MessagePack objects
- text WebSocket messages are rejected

### Client messages

`audio`

- carries one raw PCM byte chunk
- the bytes use the `pcm_encoding` and `channels` declared in the URL query string

Shape:

```json
{
  "type": "audio",
  "data": "<raw PCM bytes>"
}
```

`finish`

- marks a clean end of input
- the server replies with one final `summary` event and then closes the socket

Shape:

```json
{
  "type": "finish"
}
```

If the client disconnects without sending `finish`, the server stops processing and no final summary is guaranteed.

### Server events

`started`

- emitted once after the socket is accepted

`frame_batch`

- emitted after an audio message when that input produced one or more voiced frames
- contains the voiced frames generated from that input chunk

`chunk`

- emitted after every non-empty audio message
- contains the chunk summary for that input piece

`summary_partial`

- emitted after every non-empty audio message
- contains the current rolling overall summary

`summary`

- emitted once after `finish`
- contains the authoritative final overall summary

`error`

- emitted when the server cannot decode the MessagePack message, the PCM bytes, or the analyzer configuration
- the server closes the session after sending it

Example event shapes:

```json
{
  "type": "started",
  "backend": "raw_pcm_websocket_v2",
  "sample_rate": 16000,
  "channels": 1,
  "config": { "...": "AnalyzerConfig" }
}
```

```json
{
  "type": "frame_batch",
  "processed_samples": 4096,
  "frames": [{ "...": "FrameAnalysis" }]
}
```

```json
{
  "type": "summary",
  "processed_seconds": 1.248,
  "overall": { "...": "OverallAnalysis" }
}
```
