# Troubleshooting

Common issues when running memayu self-hosted, and how to diagnose them. The
quickest first step for almost any "it's not working" problem is `memayu doctor`,
which checks config validity, storage, and LLM/embedder connectivity in one
command:

```bash
memayu doctor
```

> **Missing config?** `memayu doctor` exits nonzero if the config can't load at
> all. Run `memayu config check` / `memayu config show` (below) to see what's
> wrong.

## Does the config load?

`memayu` reads an on-disk `config.toml` (created by `memayu setup`), then applies
any `MEMAYU_*` environment overrides. The file is resolved in this order:

1. `$MEMAYU_CONFIG`
2. `$XDG_CONFIG_HOME/memayu/config.toml`
3. `~/.config/memayu/config.toml`

If you run memayu from a service (systemd), make sure the service user can read
that file.

```bash
memayu config show     # effective config (secrets redacted)
memayu config check    # validates; exits 1 listing any problems
```

If `memayu config check` fails, fix the reported fields (e.g. a missing
`llm.base_url` or `embedder.model`) in `config.toml` or via `MEMAYU_*` env.

A missing/empty model id for a local embedder, or no dimension, produces errors
like:

```
no embedding dimension available for storage; run `memayu setup` or set MEMAYU_EMBEDDER_DIM
```

Set `MEMAYU_EMBEDDER_DIM` (or run `memayu setup` for a remote embedder) to record
the dimension.

## Is the server actually up and usable?

`memayu serve` exposes an unauthenticated readiness endpoint:

```bash
curl -s http://localhost:18080/api/health
```

- `{"status":"setup_required"}` — the process is listening but first-run setup is
  incomplete (no admin account and/or no provider config). Complete setup first.
- `{"status":"ready"}` — up and usable.

If `curl` can't connect, the server isn't listening on that address/port. Check
the bind address and port in `config.toml` (`[server]`), and that no firewall or
another process is blocking it.

## Provider problems (LLM / embedder reachable through a proxy)

`memayu doctor` verifies providers with a **real test call** (a completion for the
LLM, an embedding for the embedder), not a `/models` listing. So a provider behind
a proxy that never exposes `/models`, or renames models, still reports healthy as
long as the real call works.

If doctor reports a provider error, it prints the HTTP status or failure reason:

- **401** — the API key was rejected by the provider. Check `MEMAYU_*_API_KEY` /
  `api_key` in `config.toml`, and that the key is valid for the configured
  `base_url`.
- **500 / 5xx** — the provider (or proxy) returned a server error. Check the
  provider's own logs and that the endpoint/model combination is valid.
- **connection / timeout** — host unreachable, TLS error, or the URL is wrong.
  Confirm `base_url` is an `http(s)://` URL reachable from the memayu host, and
  that a proxy/gateway in front of the provider is up.

A dimension mismatch (configured `MEMAYU_EMBEDDER_DIM` differs from what's stored
or produced) fails doctor and will error on writes:

```
configured dimension 8 differs from stored 3
```

Set the correct consistent dimension everywhere.

## Port already in use

```bash
lsof -i :18080        # macOS / Linux
ss -tlnp | grep 18080 # Linux
```

Find and stop the process, or change `port` in the `[server]` section (and any
Docker `-p` mapping). After editing config, restart memayu.

## Reset a forgotten admin password

The web dashboard has an **Account** page (`/accounts`) for changing the password
and email when you're logged in. If the admin password is forgotten entirely and
you can't log in, reset it from the terminal (this works for both libsql and
Postgres backends):

```bash
memayu reset-password 'NewHorse-Staple-99!'
```

This bypasses the login gate, validates the new password against the same policy
the UI enforces, and writes a fresh password hash. Log in with the new password
and rotate it from `/accounts` if you like.

## API key / 401 on API calls

API routes require either an **`x-api-key`** header or session cookie. If a call
returns 401:

- Confirm the API key is valid. Generate one from the dashboard's **API Keys**
  page or via `memayu setup`. Keys are stored as a hash; the raw `mmyu_…` value is
  shown once.
- Confirm you're sending it as `x-api-key: mmyu_...`.
- `GET /api/health` is the one endpoint that needs **no** auth.

## Where are logs?

- **Terminal:** `memayu` prints diagnostics (config, embedder dimension, listening
  address) to stdout/stderr.
- **systemd:** `journalctl -u memayu -f`.
- **Docker:** `docker logs <container>`.
- Per-request HTTP logging appears in the dashboard's **Requests** page.

## Still stuck

Open an issue at https://github.com/savioruz/memayu/issues and include:
the output of `memayu doctor`, `memayu config show`, your OS/arch/build, and — for
a service — `systemctl status memayu` plus recent `journalctl -u memayu` lines.