# cors-proxy

Lightweight streaming CORS proxy in Rust (Axum + Reqwest).

- Target URL via `X-Target-URL` header
- Follows redirects (up to 10)
- Streams request/response bodies (no buffering)
- Forwards request headers (auth, cookies, content-type, multi-value, …)
- Configurable allowed origins via `ALLOWED_ORIGINS` (`*` default)
- `Dockerfile` with separate Alpine builder + Alpine runner

## Layout

```text
src/
  main.rs     # bootstrap: settings, router, serve
  config.rs   # Settings + AllowedOrigins (env parsing)
  cors.rs     # CORS header policy
  headers.rs  # request/response header forwarding + filtering
  proxy.rs    # proxy handler, target resolution, health
tests/
  conftest.py   # builds binary, spins up local upstream + proxies
  upstream.py   # hermetic upstream origin server (echo/redirect/stream/…)
  test_proxy.py # 16 pytest integration tests
```

## Run

```bash
cargo run
# or
PORT=8080 ALLOWED_ORIGINS=* cargo run
```

## Env

| Var | Default | Description |
|-----|---------|-------------|
| `PORT` | `8080` | Listen port |
| `ALLOWED_ORIGINS` | `*` | `*` or comma-separated list, e.g. `https://example.com,https://app.example.com` |
| `RUST_LOG` | `cors-proxy=info` | Log filter |

## Usage

Full target in header (path `/` is ignored):

```bash
curl -i http://localhost:8080/ \
  -H "X-Target-URL: https://api.example.com/items?page=2" \
  -H "Origin: https://myapp.com"
```

Base + path-append mode (proxy path is appended to the target):

```bash
curl -i http://localhost:8080/items?page=2 \
  -H "X-Target-URL: https://api.example.com" \
  -H "Origin: https://myapp.com"
# -> https://api.example.com/items?page=2
```

POST with body (streamed):

```bash
curl -i -X POST http://localhost:8080/ \
  -H "X-Target-URL: https://httpbin.org/post" \
  -H "Content-Type: application/json" \
  -d '{"hello":"world"}'
```

Preflight (`OPTIONS`) is answered locally with `204` + CORS headers, no upstream call.

`GET /healthz` returns `ok` (no proxying).

## CORS behavior

- `ALLOWED_ORIGINS=*`: responds `Access-Control-Allow-Origin: *`, reflects
  `Access-Control-Request-Headers`, exposes `*`, no credentials.
- `ALLOWED_ORIGINS=https://a.com,https://b.com`: echoes `Origin` only if
  allow-listed (otherwise no CORS headers), plus
  `Access-Control-Allow-Credentials: true` and `Vary: Origin`.
- Upstream `Access-Control-*` headers are stripped; only the proxy's own are sent.

## Docker

```bash
docker build -t cors-proxy .
docker run -p 8080:8080 -e ALLOWED_ORIGINS=https://myapp.com cors-proxy
```

## Tests

Unit tests (per module):

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

Integration tests (pytest, hermetic — local upstream server, no external network):

```bash
uv venv .venv
uv pip install --python .venv/bin/python -r tests/requirements.txt
.venv/bin/python -m pytest tests/ -v
```

The suite builds the debug binary once, starts a local upstream origin
server plus two proxy instances (`*` and restricted origins), and covers:
health, error cases, header forwarding (incl. multi-value), redirect
following, streaming, POST bodies, preflight, path-append mode, origin
allow-listing, and upstream CORS stripping.
