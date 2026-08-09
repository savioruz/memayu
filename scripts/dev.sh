#!/bin/sh
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [ -f "$ROOT/.env" ]; then
  set -a; . "$ROOT/.env"; set +a
  echo "[dev] loaded .env"
fi

PORT="$1"
[ -z "$PORT" ] && PORT="${MEMAYU_PORT:-18080}"

for var in MEMAYU_LLM_BASE_URL MEMAYU_LLM_MODEL MEMAYU_EMBEDDER_BASE_URL MEMAYU_EMBEDDER_MODEL; do
  if [ -z "$(eval echo \$$var)" ]; then
    echo "[dev] ERROR: $var is not set"
    exit 1
  fi
done

export MEMAYU_PORT="$PORT"

echo "[dev] watching for changes (port $PORT)"
echo "[dev] dashboard: http://localhost:$PORT"
echo "[dev] Ctrl-C to stop"

exec cargo-watch \
  --why \
  --ignore target \
  --ignore '*.db' \
  --ignore '*.db-wal' \
  --ignore '*.db-shm' \
  -w "${ROOT}/crates" \
  -w "${ROOT}/bin" \
  -s "${ROOT}/scripts/build-css.sh" \
  -x "run --manifest-path ${ROOT}/Cargo.toml --bin memayu"
