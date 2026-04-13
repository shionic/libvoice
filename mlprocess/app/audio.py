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
    cleanup_original: bool = True

    def cleanup(self) -> None:
        paths = [self.wav_path]
        if self.cleanup_original:
            paths.insert(0, self.original_path)

        for path in paths:
            try:
                path.unlink(missing_ok=True)
            except OSError:
                pass


def persist_upload_bytes(filename: str | None, data: bytes) -> Path:
    suffix = Path(filename or "upload.bin").suffix or ".bin"
    with tempfile.NamedTemporaryFile(delete=False, suffix=suffix) as handle:
        handle.write(data)
        return Path(handle.name)


def load_audio(input_path: Path) -> tuple[np.ndarray, int]:
    try:
        waveform, sample_rate = sf.read(str(input_path), always_2d=True, dtype="float32")
        if waveform.ndim != 2:
            raise ValueError("decoded waveform must have shape [samples, channels]")
        return waveform, int(sample_rate)
    except Exception:
        waveform, sample_rate = librosa.load(
            str(input_path),
            sr=None,
            mono=False,
            dtype=np.float32,
        )
        waveform = np.asarray(waveform, dtype=np.float32)
        if waveform.ndim == 1:
            waveform = waveform[:, np.newaxis]
        elif waveform.ndim == 2:
            waveform = waveform.T
        else:
            raise ValueError("decoded waveform must have shape [samples, channels]")
        return waveform, int(sample_rate)


def prepare_audio(input_path: Path, cleanup_original: bool = False) -> PreparedAudio:
    waveform, sample_rate = load_audio(input_path)

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
        cleanup_original=cleanup_original,
    )
