from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .audio import prepare_audio
from .models import ModelManager


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="python -m app.cli",
        description="Run local CPU inference with NISQA and ECAPA-TDNN on an audio file.",
    )
    parser.add_argument("audio_file", type=Path, help="Path to an input audio file.")
    parser.add_argument(
        "--no-embedding",
        action="store_true",
        help="Skip the full ECAPA embedding array and return only embedding metadata.",
    )
    parser.add_argument(
        "--indent",
        type=int,
        default=2,
        help="JSON indentation level for stdout output. Default: 2.",
    )
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    audio_file = args.audio_file.expanduser().resolve()
    if not audio_file.is_file():
        parser.error(f"audio file was not found: {audio_file}")

    model_manager = ModelManager()

    try:
        model_manager.warmup()
        prepared_audio = prepare_audio(audio_file)
    except Exception as exc:
        print(f"setup failed: {exc}", file=sys.stderr)
        return 1

    try:
        result = model_manager.analyze(prepared_audio, include_embedding=not args.no_embedding)
    except Exception as exc:
        print(f"inference failed: {exc}", file=sys.stderr)
        return 1
    finally:
        prepared_audio.cleanup()

    payload = {
        "filename": audio_file.name,
        "source_path": str(audio_file),
        **result,
    }
    json.dump(payload, sys.stdout, indent=args.indent)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

