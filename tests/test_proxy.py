"""Integration tests for the CORS proxy (live binary + local upstream)."""

import http.client
import json
from urllib.parse import urlsplit

import requests

from conftest import options_hits


def proxied(proxy, upstream, path="/echo", **kwargs):
    headers = kwargs.pop("headers", {})
    headers["X-Target-URL"] = f"{upstream}{path}"
    return requests.request("GET", f"{proxy}/", headers=headers, timeout=10, **kwargs)


def test_healthz(proxy_any):
    r = requests.get(f"{proxy_any}/healthz", timeout=5)
    assert r.status_code == 200
    assert r.text == "ok"


def test_missing_target_header_is_400_with_cors(proxy_any):
    r = requests.get(f"{proxy_any}/", timeout=5)
    assert r.status_code == 400
    assert "Missing X-Target-URL" in r.text
    assert r.headers.get("Access-Control-Allow-Origin") == "*"


def test_invalid_scheme_is_400(proxy_any):
    r = requests.get(
        f"{proxy_any}/", headers={"X-Target-URL": "ftp://example.com/x"}, timeout=5
    )
    assert r.status_code == 400


def test_get_passthrough_status_body_and_response_headers(proxy_any, upstream):
    r = proxied(proxy_any, upstream, "/created")
    assert r.status_code == 201
    assert r.json() == {"created": True}
    # Upstream response headers are forwarded …
    assert r.headers.get("X-Upstream-Test") == "yes"
    assert r.headers.get("Content-Type") == "application/json"
    # … with the proxy's own CORS headers applied.
    assert r.headers.get("Access-Control-Allow-Origin") == "*"


def test_request_headers_forwarded(proxy_any, upstream):
    r = requests.get(
        f"{proxy_any}/",
        headers={
            "X-Target-URL": f"{upstream}/echo",
            "Authorization": "Bearer secret-token",
            "X-Custom-Header": "custom-value",
            "Cookie": "session=abc123",
        },
        timeout=10,
    )
    assert r.status_code == 200
    seen = r.json()["headers"]
    assert seen.get("authorization") == "Bearer secret-token"
    assert seen.get("x-custom-header") == "custom-value"
    assert seen.get("cookie") == "session=abc123"


def test_request_multi_value_headers_forwarded(proxy_any, upstream):
    # `requests` cannot send duplicate headers, so use http.client directly.
    target = f"{upstream}/echo"
    parsed_proxy = urlsplit(proxy_any)
    conn = http.client.HTTPConnection(parsed_proxy.hostname, parsed_proxy.port, timeout=10)
    conn.putrequest("GET", "/")
    conn.putheader("X-Target-URL", target)
    conn.putheader("X-Multi", "one")
    conn.putheader("X-Multi", "two")
    conn.endheaders()
    resp = conn.getresponse()
    assert resp.status == 200
    seen = json.loads(resp.read().decode())["headers"]["x-multi"]
    conn.close()
    assert seen == ["one", "two"]


def test_target_header_not_forwarded(proxy_any, upstream):
    r = proxied(proxy_any, upstream)
    assert r.status_code == 200
    assert "x-target-url" not in r.json()["headers"]


def test_redirect_followed(proxy_any, upstream):
    r = proxied(proxy_any, upstream, "/redirect")
    assert r.status_code == 200
    assert r.json() == {"ok": True, "path": "/final"}


def test_streaming_response(proxy_any, upstream):
    expected = "".join(f"chunk-{i}\n" for i in range(5))
    with requests.get(
        f"{proxy_any}/",
        headers={"X-Target-URL": f"{upstream}/stream"},
        stream=True,
        timeout=15,
    ) as r:
        assert r.status_code == 200
        chunks = list(r.iter_content(chunk_size=8))
    assert b"".join(chunks).decode() == expected
    # Body arrived in more than one piece: it was streamed, not buffered.
    assert len(chunks) > 1


def test_post_body_forwarded(proxy_any, upstream):
    r = requests.post(
        f"{proxy_any}/",
        headers={"X-Target-URL": f"{upstream}/echo", "Content-Type": "application/json"},
        json={"hello": "world"},
        timeout=10,
    )
    assert r.status_code == 200
    payload = r.json()
    assert payload["method"] == "POST"
    assert '"hello": "world"' in payload["body"] or "hello" in payload["body"]


def test_preflight_answered_locally(proxy_any, upstream, reset_options_hits):
    r = requests.options(
        f"{proxy_any}/",
        headers={
            "X-Target-URL": f"{upstream}/echo",
            "Origin": "https://app.example.com",
            "Access-Control-Request-Method": "POST",
            "Access-Control-Request-Headers": "Content-Type, X-Custom-Header",
        },
        timeout=5,
    )
    assert r.status_code == 204
    assert r.headers.get("Access-Control-Allow-Origin") == "*"
    assert "POST" in r.headers.get("Access-Control-Allow-Methods", "")
    assert "X-Custom-Header" in r.headers.get("Access-Control-Allow-Headers", "")
    # Preflight must not hit upstream.
    assert options_hits() == 0


def test_path_append_mode(proxy_any, upstream):
    r = requests.get(
        f"{proxy_any}/echo?x=1",
        headers={"X-Target-URL": upstream},
        timeout=10,
    )
    assert r.status_code == 200
    payload = r.json()
    assert payload["method"] == "GET"
    assert payload["path"] == "/echo?x=1"


def test_restricted_origins_allowed(proxy_restricted, upstream):
    r = requests.get(
        f"{proxy_restricted}/",
        headers={"X-Target-URL": f"{upstream}/echo", "Origin": "https://allowed.com"},
        timeout=10,
    )
    assert r.status_code == 200
    assert r.headers.get("Access-Control-Allow-Origin") == "https://allowed.com"
    assert r.headers.get("Access-Control-Allow-Credentials") == "true"


def test_restricted_origins_denied(proxy_restricted, upstream):
    r = requests.get(
        f"{proxy_restricted}/",
        headers={"X-Target-URL": f"{upstream}/echo", "Origin": "https://evil.com"},
        timeout=10,
    )
    assert r.status_code == 200  # proxied fine, just no CORS headers
    assert "Access-Control-Allow-Origin" not in r.headers


def test_upstream_cors_headers_stripped_wildcard(proxy_any, upstream):
    r = requests.get(
        f"{proxy_any}/",
        headers={
            "X-Target-URL": f"{upstream}/cors-leak",
            "Origin": "https://anything.com",
        },
        timeout=10,
    )
    # Upstream sent https://upstream-evil.com; proxy must overwrite with *.
    assert r.headers.get("Access-Control-Allow-Origin") == "*"


def test_upstream_cors_headers_stripped_restricted(proxy_restricted, upstream):
    r = requests.get(
        f"{proxy_restricted}/",
        headers={
            "X-Target-URL": f"{upstream}/cors-leak",
            "Origin": "https://evil.com",
        },
        timeout=10,
    )
    # Upstream sent an ACAO header but the origin is not allow-listed,
    # so no ACAO header may be present at all.
    assert "Access-Control-Allow-Origin" not in r.headers
