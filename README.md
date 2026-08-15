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

## Usage

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

## Contributing

Issues and pull requests are welcome. Before submitting structural changes, read **[CONTRIBUTING.md](CONTRIBUTING.md)** for guidelines.

## License

MIT — see [LICENSE](LICENSE).
