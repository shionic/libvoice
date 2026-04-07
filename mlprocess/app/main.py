from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager

from fastapi import FastAPI, File, HTTPException, Query, UploadFile

from .audio import prepare_audio, persist_upload_bytes
from .models import ModelManager


model_manager = ModelManager()


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
        "models": ["nisqa", "ecapa-tdnn"],
        "routes": {"analyze": "POST /v1/analyze"},
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
        prepared_audio = await asyncio.to_thread(prepare_audio, input_path)
    except Exception as exc:
        input_path.unlink(missing_ok=True)
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
