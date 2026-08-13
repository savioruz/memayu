#!/bin/sh
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [ -f "$ROOT/.env" ]; then
  set -a; . "$ROOT/.env"; set +a
  echo "[dev] loaded .env"
fi

MODE="${1:-tui}"

for var in MEMAYU_LLM_BASE_URL MEMAYU_LLM_MODEL MEMAYU_EMBEDDER_BASE_URL MEMAYU_EMBEDDER_MODEL; do
  if [ -z "$(eval echo \$$var)" ]; then
    echo "[dev] ERROR: $var is not set"
    exit 1
  fi
done

case "$MODE" in
  tui)
    echo "[dev] watching for changes (TUI frontend)"
    echo "[dev] Ctrl-C to stop"
    exec cargo-watch \
      --why \
      --ignore target \
      --ignore '*.db' \
      --ignore '*.db-wal' \
      --ignore '*.db-shm' \
      -w "${ROOT}/crates" \
      -w "${ROOT}/bin" \
      -x "run --manifest-path ${ROOT}/Cargo.toml --bin memayu --features tui"
    ;;
  web)
    PORT="${2:-${MEMAYU_PORT:-18080}}"
    export MEMAYU_PORT="$PORT"
    echo "[dev] watching for changes (web dashboard, port $PORT)"
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
      -x "run --manifest-path ${ROOT}/Cargo.toml --bin memayu --features web -- serve"
    ;;
  *)
    echo "[dev] unknown mode: $MODE (expected tui or web)" >&2
    exit 1
    ;;
esac
