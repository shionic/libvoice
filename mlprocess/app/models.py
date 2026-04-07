from __future__ import annotations

import json
import math
import os
import sys
import urllib.request
from pathlib import Path
from typing import Any

import soundfile as sf

from .audio import PreparedAudio
from .config import Settings, settings


class ModelManager:
    def __init__(self, cfg: Settings = settings) -> None:
        self.cfg = cfg
        self._ecapa = None

    def warmup(self) -> None:
        self.cfg.models_dir.mkdir(parents=True, exist_ok=True)
        self.cfg.vendor_dir.mkdir(parents=True, exist_ok=True)
        mplconfigdir = self.cfg.models_dir / ".matplotlib"
        mplconfigdir.mkdir(parents=True, exist_ok=True)
        os.environ.setdefault("MPLCONFIGDIR", str(mplconfigdir))
        self._ensure_nisqa_repo()
        self._ensure_nisqa_weights()
        self._load_ecapa()

    def analyze(self, audio: PreparedAudio, include_embedding: bool = True) -> dict[str, Any]:
        nisqa_output = self._predict_nisqa(audio.wav_path)
        ecapa_output = self._predict_ecapa(audio.wav_path, include_embedding=include_embedding)
        return {
            "audio": {
                "duration_seconds": round(audio.duration_seconds, 6),
                "original_sample_rate": audio.original_sample_rate,
                "model_sample_rate": audio.model_sample_rate,
                "channels_after_mixdown": audio.channels_after_mixdown,
            },
            "nisqa": nisqa_output,
            "ecapa_tdnn": ecapa_output,
        }

    def _ensure_nisqa_repo(self) -> None:
        if not self.cfg.nisqa_repo_dir.exists():
            raise RuntimeError(
                "NISQA repository not found. Run mlprocess/scripts/setup_venv.sh to clone vendor/NISQA."
            )

        repo_parent = str(self.cfg.nisqa_repo_dir.resolve())
        if repo_parent not in sys.path:
            sys.path.insert(0, repo_parent)

    def _ensure_nisqa_weights(self) -> None:
        if self.cfg.nisqa_weights_path.exists():
            return

        self.cfg.nisqa_weights_path.parent.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(self.cfg.nisqa_weights_url, self.cfg.nisqa_weights_path)

    def _load_ecapa(self):
        if self._ecapa is not None:
            return self._ecapa

        import torch
        from speechbrain.inference.classifiers import EncoderClassifier

        self._ecapa = EncoderClassifier.from_hparams(
            source=self.cfg.ecapa_source,
            savedir=str(self.cfg.ecapa_savedir),
            run_opts={"device": "cpu"},
        )
        self._ecapa.device = torch.device("cpu")
        return self._ecapa

    def _predict_nisqa(self, wav_path: Path) -> dict[str, Any]:
        from nisqa.NISQA_model import nisqaModel

        args = {
            "mode": "predict_file",
            "pretrained_model": str(self.cfg.nisqa_weights_path),
            "deg": str(wav_path),
            "output_dir": None,
            "tr_bs_val": 1,
            "tr_num_workers": 0,
            "ms_channel": None,
        }
        prediction_df = nisqaModel(args).predict()
        first_row = prediction_df.iloc[0].to_dict()
        first_row = {
            key: value
            for key, value in first_row.items()
            if key not in {"deg", "filepath_deg"} and not str(key).startswith("filepath_")
        }
        return _to_jsonable(first_row)

    def _predict_ecapa(self, wav_path: Path, include_embedding: bool) -> dict[str, Any]:
        import torch

        classifier = self._load_ecapa()
        waveform, sample_rate = sf.read(str(wav_path), always_2d=True, dtype="float32")
        if sample_rate != 16_000:
            raise RuntimeError(f"expected 16 kHz normalized wav, got {sample_rate}")

        batch = torch.from_numpy(waveform.T)
        rel_length = torch.tensor([1.0], dtype=torch.float32)
        embedding = classifier.encode_batch(batch, rel_length, normalize=False).squeeze().detach().cpu()
        embedding_list = embedding.tolist()

        response = {
            "source": self.cfg.ecapa_source,
            "embedding_dim": len(embedding_list),
            "embedding_l2_norm": float(torch.linalg.vector_norm(embedding).item()),
        }
        if include_embedding:
            response["embedding"] = embedding_list
        return response


def _to_jsonable(value: Any) -> Any:
    if isinstance(value, dict):
        return {str(key): _to_jsonable(inner) for key, inner in value.items()}
    if isinstance(value, list):
        return [_to_jsonable(item) for item in value]
    if isinstance(value, Path):
        return str(value)
    if hasattr(value, "item"):
        try:
            return value.item()
        except Exception:
            pass
    if isinstance(value, float):
        if math.isnan(value) or math.isinf(value):
            return None
        return value
    if isinstance(value, (str, int, bool)) or value is None:
        return value
    try:
        json.dumps(value)
        return value
    except TypeError:
        return str(value)
