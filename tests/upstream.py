"""Local upstream origin server used as a hermetic target for proxy tests.

Endpoints (all methods accepted unless noted):
  GET /echo[?...]      -> JSON {method, path, headers, body}
                         `headers`: lower-cased name -> value or list of values.
  ANY /final           -> JSON {"ok": true, "path": "/final"}
  ANY /redirect        -> 302 to /final (tests redirect following)
  GET /stream          -> chunked plain-text "chunk-0\\n".."chunk-4\\n" with
                         small delays between chunks (tests streaming)
  ANY /created         -> 201 + header X-Upstream-Test: yes + JSON {"created": true}
  ANY /cors-leak       -> 200 + Access-Control-Allow-Origin: https://upstream-evil.com
                         (proxy must strip upstream CORS headers)
"""

from __future__ import annotations

import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class UpstreamHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    # Count OPTIONS hits so tests can assert preflight never reaches upstream.
    options_hits = 0
    options_hits_lock = threading.Lock()

    def log_message(self, *args):  # silence noisy logging during tests
        pass

    def _headers_dict(self):
        headers: dict[str, object] = {}
        for key in self.headers.keys():
            values = self.headers.get_all(key)
            lowered = key.lower()
            if values is not None and len(values) > 1:
                headers[lowered] = values
            else:
                headers[lowered] = self.headers[key]
        return headers

    def _read_body(self) -> str:
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            return self._read_chunked()
        length = self.headers.get("Content-Length")
        if not length:
            return ""
        try:
            n = int(length)
        except ValueError:
            return ""
        if n <= 0:
            return ""
        return self.rfile.read(n).decode("utf-8", errors="replace")

    def _read_chunked(self) -> str:
        chunks: list[bytes] = []
        while True:
            line = self.rfile.readline(8192).decode("ascii", errors="replace")
            size_str = line.split(";", 1)[0].strip()
            try:
                size = int(size_str, 16)
            except ValueError:
                break
            if size == 0:
                self.rfile.readline(8192)  # trailing CRLF + footers
                break
            chunks.append(self.rfile.read(size))
            self.rfile.readline(8192)  # trailing CRLF
        return b"".join(chunks).decode("utf-8", errors="replace")

    def _send_json(self, status: int, obj: object, extra: dict | None = None):
        payload = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)

    def _handle(self):
        path = self.path.split("?", 1)[0]
        if path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "/final")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if path == "/final":
            self._send_json(200, {"ok": True, "path": "/final"})
            return
        if path == "/stream":
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            # Close-delimited body: the proxy re-streams it to the client.
            self.send_header("Connection", "close")
            self.end_headers()
            for i in range(5):
                self.wfile.write(f"chunk-{i}\n".encode())
                self.wfile.flush()
                time.sleep(0.05)
            self.close_connection = True
            return
        if path == "/created":
            self._send_json(201, {"created": True}, {"X-Upstream-Test": "yes"})
            return
        if path == "/cors-leak":
            self._send_json(
                200,
                {"ok": True},
                {"Access-Control-Allow-Origin": "https://upstream-evil.com"},
            )
            return
        if path == "/echo":
            self._send_json(
                200,
                {
                    "method": self.command,
                    "path": self.path,
                    "headers": self._headers_dict(),
                    "body": self._read_body(),
                },
            )
            return
        self._send_json(404, {"error": "not found", "path": self.path})

    do_GET = _handle
    do_POST = _handle
    do_PUT = _handle
    do_PATCH = _handle
    do_DELETE = _handle
    do_HEAD = _handle

    def do_OPTIONS(self):
        with self.options_hits_lock:
            UpstreamHandler.options_hits += 1
        self._handle()


def start_upstream_server() -> tuple[ThreadingHTTPServer, threading.Thread, str]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), UpstreamHandler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, thread, f"http://127.0.0.1:{port}"
