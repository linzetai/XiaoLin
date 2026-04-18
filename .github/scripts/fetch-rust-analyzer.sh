#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "Usage: $0 <rust-target-triple> <output-dir>" >&2
  exit 1
fi

TARGET="$1"
OUT_DIR="$2"
mkdir -p "$OUT_DIR"

TMP_DIR="$(mktemp -d)"
RA_TAG="${RUST_ANALYZER_TAG:-latest}"
if [[ "${RA_TAG}" == "latest" ]]; then
  URL="https://github.com/rust-lang/rust-analyzer/releases/latest/download/rust-analyzer-${TARGET}.gz"
else
  URL="https://github.com/rust-lang/rust-analyzer/releases/download/${RA_TAG}/rust-analyzer-${TARGET}.gz"
fi

ARCHIVE_NAME="rust-analyzer-${TARGET}.gz"
ARCHIVE_PATH="${TMP_DIR}/${ARCHIVE_NAME}"

echo "Downloading ${URL}"
curl -fL --retry 5 --retry-delay 2 --connect-timeout 15 --max-time 600 \
  -o "${ARCHIVE_PATH}" "${URL}"

python3 - <<'PY' "${ARCHIVE_PATH}" "${OUT_DIR}"
import gzip
import os
import stat
import sys

archive_path = sys.argv[1]
out_dir = sys.argv[2]

binary_name = "rust-analyzer.exe" if os.name == "nt" else "rust-analyzer"
destination = os.path.join(out_dir, binary_name)
with gzip.open(archive_path, "rb") as src, open(destination, "wb") as dst:
    dst.write(src.read())

if os.name != "nt":
    mode = os.stat(destination).st_mode
    os.chmod(destination, mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

print(destination)
PY

rm -rf "${TMP_DIR}"
echo "rust-analyzer bundled into ${OUT_DIR}"
