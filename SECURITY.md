# Security Policy

## Reporting a vulnerability

Memayu is a self-hosted memory engine — it stores users' personal facts, so security matters.

Please **do not** open a public issue for security vulnerabilities. Instead, report privately:

- Email: **kheril@svrz.xyz** (or DM the maintainer on GitHub)

You should receive a response within 48 hours. If not, follow up.

## What to include

- Affected version/commit
- Reproduction steps or a minimal PoC
- Impact assessment

## Scope

- The `memayu` self-hosted binary (Rust workspace in this repo)
- API key handling, encrypted credential storage, and any data-leak paths

## Disclosure

We prefer coordinated disclosure: report first, then allow a reasonable window before public disclosure.
