# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-25

### Added

- Local embedding backend via Candle (#18)
- Web account management: change password / email (#50, #64)
- Setup wizard CLI-interactive + --tui, admin + API key (#54)
- Batch endpoint for memories HTTP+MCP+CLI (#53)
- /health setup_required vs ready (#47)
- systemd unit + docker-compose + troubleshooting docs (#49, #65)

### Fixed

- Load .env file (was OS env only) (#44)
- TUI falls back to serve mode on non-TTY (#45)
- MCP initialize camelCase (#52)
- Setup API field names consistent (#51)
- doctor false-positive on proxied providers (#48)

### Changed

- Web memory detail modal (#56, #63)

## [0.1.0-rc2] - 2026-08-15

### Added

- XDG config file + first-run setup wizard (#16, closes #2)
- Ratatui TUI as default frontend (#25, closes #3)
- Hybrid search: vector + full-text with RRF fusion (Postgres tsvector/tsquery,
  libSQL FTS5), API contract unchanged (#27, closes #20)
- Raw extraction mode (`extraction_mode = raw`, stores verbatim, skips LLM) (#30, closes #28)
- Metadata filtering and cursor pagination for search/list (#41, closes #8)
- CLI memory management: memayu add/search/list/delete (#42, closes #7)
- Doctor diagnostics: memayu doctor (#42, closes #5)
- cargo-audit CI dependency scanning (#26, closes #24)
- Hybrid search fusion tests (#36)
- Expanded conflict resolution test suite (#27, closes #21)

### Fixed

- True cosine similarity computation, threshold lowered to 0.65 (#14, closes #1)
- Metadata preserved in all API responses (#15, closes #10)
- Blank /docs and broken post-login home (#31)
- Shared identity resolution across TUI, Web, MCP — no more hardcoded "default"
  user_id; one-time migration backfills orphans (#34, closes #32)
- FTS5 syntax-error crash on special chars ($ ! " * : ^ AND/OR/NOT) (#37)
- Postgres null and score-type issues in request logs / full-text search (#39)
- FTS5 tokenizer: porter+unicode61 stemming, "work" matches "works" (#40, closes #38)

### Changed

- Security hardening: rate limiting, security headers, CORS, HTTP client timeouts (#17, closes #4)
- Trimmed libsql features to reduce binary size (#26)

## [0.1.0-rc1] - 2026-08-11

Initial pre-release. First public build of Memayu — a local-first, agent memory
engine with vector-based retrieval, OpenAI-compatible LLM/embedder support, and a
Rust core.

[Unreleased]: https://github.com/savioruz/memayu/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/savioruz/memayu/compare/v0.1.0-rc2...v0.1.0
[0.1.0-rc2]: https://github.com/savioruz/memayu/compare/v0.1.0-rc1...v0.1.0-rc2
[0.1.0-rc1]: https://github.com/savioruz/memayu/releases/tag/v0.1.0-rc1
