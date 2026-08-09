# Contributing

Thanks for your interest in Memayu. It's a small, solo-maintained project, so a few guidelines help keep things moving.

## Getting started

- Fork the repo and create a feature branch.
- PRs target `main`.
- Run `make setup` once after cloning to enable the pre-commit hooks (`.githooks/pre-commit`). It just runs `git config core.hooksPath .githooks`.

## Before you submit

Everything below must pass — CI enforces it too:

```bash
make fmt-check   # formatting
make lint        # clippy, warnings are errors
make test        # all tests, all features
```

## Guidelines

- **Follow ARCHITECTURE.md.** The dependency rules are the core design guarantee: `memayu-core` must never depend on a concrete implementation, and nothing may depend on `memayu-cloud`. Don't leak boundaries.
- **One concern per crate/file.** If a file is growing, it's likely doing too much.
- **Add tests with behavior.** The ADD vs UPDATE logic and dimension guard are the differentiators — changes to them need tests.
- **Write small PRs.** Easier to review, easier to get merged.
- **Docs live in the Obsidian vault**, not the repo. Code comments should explain *why*, not restate the code.

## Commit messages

Concise, imperative mood: `fix: memory search scope`, `feat(api): add list endpoint`.

## Open questions

Check the PRDs' "Open Questions" sections (`projects/memayu/`) — unresolved design decisions are tracked there. If your PR answers one, update the doc too.
