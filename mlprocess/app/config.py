from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


BASE_DIR = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class Settings:
    host: str = os.getenv("MLPROCESS_HOST", "0.0.0.0")
    port: int = int(os.getenv("MLPROCESS_PORT", "8010"))
    models_dir: Path = Path(os.getenv("MLPROCESS_MODELS_DIR", BASE_DIR / "models")).resolve()
    vendor_dir: Path = Path(os.getenv("MLPROCESS_VENDOR_DIR", BASE_DIR / "vendor")).resolve()
    ecapa_source: str = os.getenv("MLPROCESS_ECAPA_SOURCE", "speechbrain/spkrec-ecapa-voxceleb")
    ecapa_savedir: Path = Path(
        os.getenv("MLPROCESS_ECAPA_SAVEDIR", BASE_DIR / "models" / "ecapa-tdnn")
    ).resolve()
    nisqa_repo_dir: Path = Path(
        os.getenv("MLPROCESS_NISQA_REPO_DIR", BASE_DIR / "vendor" / "NISQA")
    ).resolve()
    nisqa_weights_path: Path = Path(
        os.getenv("MLPROCESS_NISQA_WEIGHTS_PATH", BASE_DIR / "models" / "nisqa.tar")
    ).resolve()
    nisqa_weights_url: str = os.getenv(
        "MLPROCESS_NISQA_WEIGHTS_URL",
        "https://raw.githubusercontent.com/gabrielmittag/NISQA/master/weights/nisqa.tar",
    )


settings = Settings()

