from __future__ import annotations

import tempfile
from dataclasses import dataclass
from pathlib import Path

import librosa
import numpy as np
import soundfile as sf


MODEL_SAMPLE_RATE = 16_000


@dataclass
class PreparedAudio:
    original_path: Path
    wav_path: Path
    original_sample_rate: int
    model_sample_rate: int
    duration_seconds: float
    channels_after_mixdown: int

    def cleanup(self) -> None:
        for path in (self.original_path, self.wav_path):
            try:
                path.unlink(missing_ok=True)
            except OSError:
                pass


def persist_upload_bytes(filename: str | None, data: bytes) -> Path:
    suffix = Path(filename or "upload.bin").suffix or ".bin"
    with tempfile.NamedTemporaryFile(delete=False, suffix=suffix) as handle:
        handle.write(data)
        return Path(handle.name)


def prepare_audio(input_path: Path) -> PreparedAudio:
    waveform, sample_rate = sf.read(str(input_path), always_2d=True, dtype="float32")
    if waveform.ndim != 2:
        raise ValueError("decoded waveform must have shape [samples, channels]")

    mono_waveform = np.mean(waveform, axis=1, dtype=np.float32)
    if sample_rate != MODEL_SAMPLE_RATE:
        mono_waveform = librosa.resample(
            mono_waveform,
            orig_sr=sample_rate,
            target_sr=MODEL_SAMPLE_RATE,
        ).astype(np.float32, copy=False)

    duration_seconds = 0.0
    if mono_waveform.size > 0:
        duration_seconds = mono_waveform.shape[0] / float(MODEL_SAMPLE_RATE)

    with tempfile.NamedTemporaryFile(delete=False, suffix=".wav") as handle:
        wav_path = Path(handle.name)

    sf.write(str(wav_path), mono_waveform, MODEL_SAMPLE_RATE, format="WAV", subtype="PCM_16")

    return PreparedAudio(
        original_path=input_path,
        wav_path=wav_path,
        original_sample_rate=sample_rate,
        model_sample_rate=MODEL_SAMPLE_RATE,
        duration_seconds=duration_seconds,
        channels_after_mixdown=1,
    )
