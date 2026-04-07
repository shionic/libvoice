#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PYTHON_BIN="${1:-}"

if [[ -z "${PYTHON_BIN}" ]]; then
  for candidate in python3.12 python3.11 python3.10; do
    if command -v "${candidate}" >/dev/null 2>&1; then
      PYTHON_BIN="${candidate}"
      break
    fi
  done
fi

if [[ -z "${PYTHON_BIN}" ]]; then
  echo "No supported Python interpreter found. Install Python 3.10, 3.11, or 3.12." >&2
  exit 1
fi

if ! command -v "${PYTHON_BIN}" >/dev/null 2>&1; then
  echo "Interpreter '${PYTHON_BIN}' was not found in PATH." >&2
  exit 1
fi

mkdir -p "${ROOT_DIR}/models" "${ROOT_DIR}/vendor"

"${PYTHON_BIN}" -m venv "${ROOT_DIR}/.venv"
source "${ROOT_DIR}/.venv/bin/activate"

python -m pip install --upgrade pip setuptools wheel
python -m pip install -r "${ROOT_DIR}/requirements.txt"

if [[ ! -d "${ROOT_DIR}/vendor/NISQA/.git" ]]; then
  git clone --depth 1 https://github.com/gabrielmittag/NISQA.git "${ROOT_DIR}/vendor/NISQA"
fi

if [[ ! -f "${ROOT_DIR}/models/nisqa.tar" ]]; then
  curl -L \
    https://raw.githubusercontent.com/gabrielmittag/NISQA/master/weights/nisqa.tar \
    -o "${ROOT_DIR}/models/nisqa.tar"
fi

echo "mlprocess setup complete."

