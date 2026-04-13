from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import BackgroundTasks, FastAPI, File, HTTPException, Query, UploadFile
from fastapi.responses import FileResponse

from .audio import prepare_audio, persist_upload_bytes
from .models import ModelManager


model_manager = ModelManager()


def _safe_unlink(path: Path) -> None:
    path.unlink(missing_ok=True)


@asynccontextmanager
async def lifespan(_: FastAPI):
    await asyncio.to_thread(model_manager.warmup)
    yield


app = FastAPI(title="mlprocess", version="0.1.0", lifespan=lifespan)


@app.get("/")
async def health() -> dict[str, object]:
    return {
        "service": "mlprocess",
        "status": "ok",
        "models": ["nisqa", "ecapa-tdnn", model_manager.cfg.demucs_bundle],
        "routes": {
            "analyze": "POST /v1/analyze",
            "remove_music": "POST /v1/remove-music",
        },
    }


@app.post("/v1/analyze")
async def analyze(
    file: UploadFile = File(...),
    include_embedding: bool = Query(True, description="Include full ECAPA embedding vector in response."),
) -> dict[str, object]:
    payload = await file.read()
    if not payload:
        raise HTTPException(status_code=400, detail="uploaded file is empty")

    input_path = persist_upload_bytes(file.filename, payload)

    try:
        prepared_audio = await asyncio.to_thread(prepare_audio, input_path, True)
    except Exception as exc:
        _safe_unlink(input_path)
        raise HTTPException(status_code=400, detail=f"failed to decode audio: {exc}") from exc

    try:
        result = await asyncio.to_thread(model_manager.analyze, prepared_audio, include_embedding)
    except Exception as exc:
        raise HTTPException(status_code=500, detail=f"model inference failed: {exc}") from exc
    finally:
        prepared_audio.cleanup()

    return {
        "filename": file.filename,
        "content_type": file.content_type,
        **result,
    }


@app.post("/v1/remove-music")
async def remove_music(
    background_tasks: BackgroundTasks,
    file: UploadFile = File(...),
) -> FileResponse:
    payload = await file.read()
    if not payload:
        raise HTTPException(status_code=400, detail="uploaded file is empty")

    input_path = persist_upload_bytes(file.filename, payload)

    try:
        result = await asyncio.to_thread(model_manager.remove_music, input_path)
    except Exception as exc:
        _safe_unlink(input_path)
        raise HTTPException(status_code=500, detail=f"music removal failed: {exc}") from exc

    background_tasks.add_task(_safe_unlink, input_path)
    background_tasks.add_task(result.cleanup)

    input_stem = Path(file.filename or "upload").stem
    output_name = f"{input_stem}-vocals.wav"

    return FileResponse(
        path=result.output_path,
        media_type="audio/wav",
        filename=output_name,
        background=background_tasks,
        headers={
            "X-MLProcess-Model": result.model_name,
            "X-MLProcess-Stem": result.stem_name,
            "X-MLProcess-Output-Sample-Rate": str(result.output_sample_rate),
            "X-MLProcess-Output-Channels": str(result.output_channels),
            "X-MLProcess-Duration-Seconds": f"{result.duration_seconds:.6f}",
        },
    )
