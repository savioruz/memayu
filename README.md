# Memayu

> Lightweight, self-hosted-first AI agent memory engine.
>
> *From "memayu hayuning bawana" (Javanese philosophy, roughly "to beautify the beauty of the world").*

[![CI](https://github.com/savioruz/memayu/actions/workflows/ci.yml/badge.svg)](https://github.com/savioruz/memayu/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![GitHub release (latest SemVer)](https://img.shields.io/github/v/release/savioruz/memayu)](https://github.com/savioruz/memayu/releases)

## Why Memayu

- **Self-hosted, free forever.** No license gating, no per-seat pricing. BYOK (bring your own keys) for LLM and embedding providers — you own your data end to end.
- **Ships as a single static binary.** Written in Rust, no runtime dependencies. Runs comfortably on a $5 VPS or a Raspberry Pi.
- **Auto-detects embedding dimension mismatches.** No more `expected 1536, got 768` debugging sessions. Memayu probes your embedder on startup and configures itself.
- **Local embedding (no API key).** An in-process Candle backend runs a multilingual model fully on-device with zero network calls at inference time — a pure-Rust single-binary alternative to BYOK, ideal for Raspberry Pi and offline VPS setups. The default model is `paraphrase-multilingual-MiniLM-L12-v2` (384-d, covers Bahasa Indonesia + English).
- **ADD vs UPDATE extraction.** New facts replace conflicting old facts automatically — your agent's memory stays a coherent knowledge base, not a growing append-only log.
- **Raw mode.** Set `MEMAYU_EXTRACTION_MODE=raw` (or `behavior.extraction_mode = "raw"` in the config file) to skip LLM extraction and store memories verbatim, with aggressive (0.98) deduplication — ideal for notes, logs, and low-latency ingestion.
- **Hybrid search.** Vector similarity fused with full-text retrieval (libSQL FTS5 / Postgres tsvector) via Reciprocal Rank Fusion (RRF), so exact keyword matches and semantic matches both surface.
- **`memayu doctor`.** Built-in diagnostics for self-hosted troubleshooting — validates config, storage, LLM, and embedder connectivity in one command.
- **Non-interactive CLI.** `memayu add`, `memayu search`, `memayu list`, and `memayu delete` work in-process for scripting without a server.

## Quick Start

### Install via curl

```bash
curl -fsSL https://raw.githubusercontent.com/savioruz/memayu/main/install.sh | sh
```

Or grab a binary directly from the [releases page](https://github.com/savioruz/memayu/releases).

### Build from source

```bash
cargo build --release
./target/release/memayu
```

This starts the Ratatui terminal UI (the default frontend). To run the web
dashboard instead, build with all features and use the `serve` subcommand:

```bash
cargo build --release --all-features
./target/release/memayu serve
```

### Docker

```bash
docker run --rm -p 8080:8080 \
  -e MEMAYU_LLM_BASE_URL=https://api.openai.com/v1 \
  -e MEMAYU_LLM_API_KEY=sk-... \
  -e MEMAYU_LLM_MODEL=gpt-4o-mini \
  -e MEMAYU_EMBEDDER_BASE_URL=https://api.openai.com/v1 \
  -e MEMAYU_EMBEDDER_API_KEY=sk-... \
  -e MEMAYU_EMBEDDER_MODEL=text-embedding-3-small \
  -v $PWD/data:/data \
  -e MEMAYU_LIBSQL_PATH=/data/memayu.db \
  ghcr.io/savioruz/memayu:latest
```

The server exposes an unauthenticated readiness endpoint at
`GET /api/health`:

```json
{ "status": "setup_required" }
```

`setup_required` means the process is listening but not yet usable (first-run
setup is incomplete — no admin account and/or no provider config). Once both
exist it returns `{ "status": "ready" }`. Use this as the healthcheck target
for Docker/systemd instead of treating "port is listening" as "server is
usable".

**Docker HEALTHCHECK** (add to a `Dockerfile` or `docker-compose.yml`):

```dockerfile
HEALTHCHECK --interval=5s --timeout=3s --start-period=10s --retries=5 \
  CMD wget -q -O /dev/stdout http://127.0.0.1:8080/api/health | grep -q '"status":"ready"'
```

**systemd** (ExecStartPost waits for readiness before the unit is active):

```ini
ExecStartPost=/bin/sh -c 'for i in $(seq 1 30); do \
  curl -fsS http://127.0.0.1:18080/api/health | grep -q "\"status\":\"ready\"" && exit 0; \
  sleep 1; done; exit 1'
```

## Usage

### CLI

`memayu` with no subcommand starts the default frontend — the Ratatui TUI when
compiled in, otherwise the web dashboard. In a headless or piped invocation
(no TTY), it detects that a TUI cannot render and falls back to `serve` mode
instead of hanging.

First-time setup is a guided wizard:

```bash
memayu setup            # interactive CLI wizard (plain stdin/stdout)
memayu setup --tui      # the same flow, rendered as a ratatui TUI
```

Both presenters ask the identical set of questions in the same order: device
check, storage backend, embedder backend, extraction mode, admin email +
password, and bind address/port. The first step probes the machine (OS, CPU
architecture, RAM, free disk) and reports whether on-device embedding is
viable. When it is, the "embedder backend" step offers `local` and, on choosing
it, a picker among four bundled Candle models (all-MiniLM-L6-v2,
bge-small-en-v1.5, paraphrase-multilingual-MiniLM-L12-v2, nomic-embed-text-v1.5)
with their dimensions, sizes, memory/disk footprint, CPU notes, and supported
languages. When local embedding is not viable (32-bit ARM, or too little
RAM/disk), the `local` option is withheld and the HTTP embedder is used
instead. On completion memayu writes `config.toml`, creates the admin account,
and prints a fresh `mmyu_…` API key exactly once. The CLI wizard reads from
plain stdin/stdout, so it also works with piped input (agent-friendly) and no
TTY. If a config file already exists, the wizard pre-fills its values as
defaults for re-configuration. In the TUI wizard, `Enter`/`Tab`/`→` advance to
the next field (submitting the current step on the last field), `←` moves back
to the previous field or step, `↑`/`↓` pick a select option, and `Esc`/`Ctrl-C`
cancels.

Other subcommands: `memayu config show|check`, `memayu add`, `memayu search`,
`memayu list`, `memayu get`, `memayu delete`, `memayu doctor`, plus
`memayu serve` (web, `web` feature) and `memayu mcp` (`mcp` feature).

### HTTP API

All `/api/memories/*` routes require authentication via `x-api-key` header or session cookie.

```bash
# Add a memory
curl -X POST http://localhost:8080/api/memories/add \
  -H 'x-api-key: YOUR_API_KEY' \
  -H 'content-type: application/json' \
  -d '{"content": "User lives in Jakarta", "metadata": {}}'

# Response
# {"result": {"status": "success", "memory_id": "abc123...", "dimension": 1536}}

# Search memories by semantic similarity
curl -X POST http://localhost:8080/api/memories/search \
  -H 'x-api-key: YOUR_API_KEY' \
  -H 'content-type: application/json' \
  -d '{"query": "where does the user live", "limit": 5}'

# Response
# {"result": {"memories": [
#   {"memory_id": "abc...", "content": "User lives in Jakarta", "score": 0.92, "created_at": "2026-..."}
# ]}}

# List all memories (limit defaults to 50, hard max 100)
curl 'http://localhost:8080/api/memories/list?limit=50' \
  -H 'x-api-key: YOUR_API_KEY'

# Response
# {"result": {"memories": [...], "next_cursor": "abc..." | null, "total_data": 42}}

# Delete a memory
curl -X DELETE 'http://localhost:8080/api/memories/{id}' \
  -H 'x-api-key: YOUR_API_KEY'

# Update a memory's content
curl -X PATCH 'http://localhost:8080/api/memories/{id}' \
  -H 'x-api-key: YOUR_API_KEY' \
  -H 'content-type: application/json' \
  -d '{"content": "User moved to Bandung"}'
```

### MCP (Model Context Protocol)

Memayu ships an MCP stdio server as `memayu mcp`. It auto-detects local vs. cloud mode based on environment.

**Self-hosted (in-process):**

```json
{
  "mcpServers": {
    "memayu": {
      "command": "memayu",
      "args": ["mcp"],
      "env": {
        "MEMAYU_STORAGE_BACKEND": "libsql",
        "MEMAYU_LIBSQL_PATH": "./memayu.db",
        "MEMAYU_LLM_BASE_URL": "https://api.openai.com/v1",
        "MEMAYU_LLM_API_KEY": "sk-...",
        "MEMAYU_LLM_MODEL": "gpt-4o-mini",
        "MEMAYU_EMBEDDER_BASE_URL": "https://api.openai.com/v1",
        "MEMAYU_EMBEDDER_API_KEY": "sk-...",
        "MEMAYU_EMBEDDER_MODEL": "text-embedding-3-small"
      }
    }
  }
}
```

**Cloud (remote API):**

```json
{
  "mcpServers": {
    "memayu": {
      "command": "memayu",
      "args": ["mcp"],
      "env": {
        "MEMAYU_API_URL": "https://your-memayu-instance.example.com",
        "MEMAYU_API_KEY": "mk_..."
      }
    }
  }
}
```

## Comparison

| | Memayu | mem0 | Zep | Letta | agentmemory |
|---|---|---|---|---|---|
| **Language** | Rust | Python | Go / Python | Python | Python |
| **Binary size** | ~15 MB static | N/A (Python runtime) | N/A | N/A | N/A |
| **Self-hosted** | Yes (single binary) | Yes (Docker / pip) | Yes (Docker) | Yes (Docker / pip) | Yes (pip) |
| **Free forever** | Yes, MIT | Yes (Apache 2.0) | Limited (Community tier) | Yes (Apache 2.0) | Yes (MIT) |
| **Embedding dim auto-detect** | Yes | No | No | No | No |
| **ADD vs UPDATE extraction** | Yes | N/A | Configurable | Via archival memory | No |
| **Postgres** | Yes (`pgvector`) | Yes | Required | Required | No |
| **Embedded SQL** | Yes (`libsql`) | SQLite (via Python) | No | No | No |
| **MCP stdio** | Yes | Community adapter | No | Planned | No |
| **BYOK** | Yes | Yes | Yes | OpenAI-only default | Yes |

> *Benchmark pending* — RAM usage, latency, and throughput comparisons have not been measured yet. This table reflects feature parity, not performance claims.

## Architecture

Memayu follows a ports-and-adapters architecture with a strict dependency rule: `memayu-core` defines the domain logic and traits; nothing in core depends on any concrete implementation. Storage (libsql, postgres+pgvector), LLM clients, and transport layers (HTTP API, MCP stdio) are separate crates wired together in the binary.

## Configuration

Configuration loads from an optional TOML file at `~/.config/memayu/config.toml` (or `$XDG_CONFIG_HOME/memayu/config.toml`), overridable via the `MEMAYU_CONFIG` env var. Environment variables (prefixed `MEMAYU_`) remain supported and override the file. See **[.env.example](.env.example)** for the complete reference with defaults and descriptions.

### Local embedding (no API key)

The embedder can run fully on-device instead of calling a remote API. Set the embedder backend to `local` in the config file:

```toml
[embedder]
backend = "local"               # "local" (on-device Candle) or "remote" (BYOK, default)
model   = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
```

or via environment variables: `MEMAYU_EMBEDDER_BACKEND=local` and `MEMAYU_EMBEDDER_MODEL=<HF model id>`.

- **No API key.** A local backend needs no `base_url` or `api_key`; nothing is sent over the network at inference time.
- **One-time download.** The model weights are downloaded from Hugging Face on first use and cached under the local data directory (override with `MEMAYU_MODEL_DIR`). Subsequent runs are fully offline.
- **Default model.** `paraphrase-multilingual-MiniLM-L12-v2` (384-d) is multilingual, so it handles a Bahasa Indonesia + English technical mix out of the box — measured recall@3 of 5/5 (100%) on a fixed mixed corpus in the local e2e suite. `all-MiniLM-L6-v2` or another HF sentence-transformer id can be substituted for English-only users.
- **Dimension auto-detect.** The local model's output dimension is probed the same way as remote providers, so no manual dimension config is needed.
- **Wizard & dashboard.** `memayu setup` offers `local` as the default embedder backend, and the web dashboard (`/providers`) exposes a backend selector. `memayu doctor` reports the active backend and skips HTTP probes when the backend is local.
- **Build.** The Candle backend is compiled in by default (`memayu-llm-client`'s `local-embedding` feature). Use `--no-default-features` on that crate for a smaller HTTP-only build.

## Contributing

Issues and pull requests are welcome. Before submitting structural changes, read **[CONTRIBUTING.md](CONTRIBUTING.md)** for guidelines.

## License

MIT — see [LICENSE](LICENSE).
