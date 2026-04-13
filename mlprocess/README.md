# mlprocess

`mlprocess` is a Python FastAPI service that runs CPU audio pipelines for uploaded audio:

- `NISQA` for non-intrusive speech quality assessment
- `ECAPA-TDNN` for speaker embeddings via SpeechBrain
- `HDemucs` for vocal/music source separation

The service accepts `multipart/form-data` uploads. The analysis route normalizes input to mono 16 kHz WAV internally for NISQA and ECAPA. Music removal uses a separate HDemucs endpoint and returns a vocals-only WAV file.

## Important runtime note

The current machine reports `Python 3.14.3`. PyTorch and related speech packages are typically not published for Python 3.14 yet, so use a `venv` created from Python `3.11` or `3.12`.

## Setup

```bash
cd /home/shione/projects/rust/voicelib/mlprocess
./scripts/setup_venv.sh python3.12
```

If you omit the interpreter, the script tries `python3.12`, then `python3.11`, then `python3.10`.

What the setup script does:

- creates `.venv`
- installs Python dependencies from `requirements.txt`
- clones the official NISQA repository into `vendor/NISQA`
- downloads `nisqa.tar` into `models/nisqa.tar`

## Run

```bash
cd /home/shione/projects/rust/voicelib/mlprocess
source .venv/bin/activate
uvicorn app.main:app --host 0.0.0.0 --port 8010
```

## CLI

Run analysis directly on a local file:

```bash
cd /home/shione/projects/rust/voicelib/mlprocess
source .venv/bin/activate
python -m app.cli /path/to/sample.wav
```

To omit the full ECAPA embedding vector:

```bash
python -m app.cli /path/to/sample.wav --no-embedding
```

To remove background music and write a vocals-only WAV:

```bash
python -m app.cli /path/to/song.mp3 --remove-music-out /tmp/song-vocals.wav
```

## API

### Health

```bash
curl http://127.0.0.1:8010/
```

### Analyze audio

```bash
curl \
  -X POST \
  "http://127.0.0.1:8010/v1/analyze?include_embedding=true" \
  -F "file=@sample.wav"
```

### Remove music from a song

This returns a `audio/wav` file containing the HDemucs `vocals` stem.

```bash
curl \
  -X POST \
  "http://127.0.0.1:8010/v1/remove-music" \
  -F "file=@song.mp3" \
  --output song-vocals.wav
```

### Example response

```json
{
  "filename": "sample.wav",
  "content_type": "audio/wav",
  "audio": {
    "duration_seconds": 2.31,
    "original_sample_rate": 48000,
    "model_sample_rate": 16000,
    "channels_after_mixdown": 1
  },
  "nisqa": {
    "filepath_deg": "/tmp/tmpabc123.wav",
    "mos_pred": 3.72,
    "noi_pred": 4.11,
    "col_pred": 3.85,
    "dis_pred": 4.02,
    "loud_pred": 3.67
  },
  "ecapa_tdnn": {
    "source": "speechbrain/spkrec-ecapa-voxceleb",
    "embedding_dim": 192,
    "embedding_l2_norm": 8.42,
    "embedding": [0.01, -0.04, 0.08]
  }
}
```

## Environment variables

- `MLPROCESS_HOST` default `0.0.0.0`
- `MLPROCESS_PORT` default `8010`
- `MLPROCESS_MODELS_DIR` default `./models`
- `MLPROCESS_VENDOR_DIR` default `./vendor`
- `MLPROCESS_TORCH_HOME` default `./models/torch-cache`
- `MLPROCESS_ECAPA_SOURCE` default `speechbrain/spkrec-ecapa-voxceleb`
- `MLPROCESS_ECAPA_SAVEDIR` default `./models/ecapa-tdnn`
- `MLPROCESS_DEMUCS_BUNDLE` default `HDEMUCS_HIGH_MUSDB_PLUS`
- `MLPROCESS_NISQA_REPO_DIR` default `./vendor/NISQA`
- `MLPROCESS_NISQA_WEIGHTS_PATH` default `./models/nisqa.tar`
- `MLPROCESS_NISQA_WEIGHTS_URL` default official raw GitHub URL for `weights/nisqa.tar`

## Notes

- CPU-only inference is enforced in application code.
- HDemucs runs against the `torchaudio` pretrained bundle and writes its model cache under `MLPROCESS_TORCH_HOME`, which avoids read-only home-directory cache issues in sandboxed environments.
- For compressed formats such as MP3/M4A/OGG, decoding support depends on the installed `torchaudio` backend and codecs available on the host.
- NISQA’s upstream repository was originally published with an older Python stack, so if installation fails on your target machine, the first thing to check is the Python version and PyTorch compatibility.

## Upstream references

- NISQA official repository: <https://github.com/gabrielmittag/NISQA>
- SpeechBrain ECAPA-TDNN model card: <https://huggingface.co/speechbrain/spkrec-ecapa-voxceleb>
- SpeechBrain classifier API: <https://speechbrain.readthedocs.io/en/latest/API/speechbrain.inference.classifiers.html>
- Torchaudio HDemucs bundle docs: <https://pytorch.org/audio/stable/pipelines.html#source-separation>
