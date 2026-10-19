"""Shared fixtures: build the proxy binary, start local upstream + proxies."""

from __future__ import annotations

import os
import socket
import subprocess
import time
from pathlib import Path

import pytest
import requests

from upstream import UpstreamHandler, start_upstream_server

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "debug" / "cors-proxy"


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_health(base_url: str, timeout: float = 20.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            r = requests.get(f"{base_url}/healthz", timeout=1)
            if r.status_code == 200 and r.text == "ok":
                return
        except requests.RequestException:
            pass
        time.sleep(0.1)
    raise RuntimeError(f"proxy at {base_url} never became healthy")


def start_proxy(port: int, allowed_origins: str) -> subprocess.Popen:
    env = os.environ.copy()
    env["PORT"] = str(port)
    env["ALLOWED_ORIGINS"] = allowed_origins
    proc = subprocess.Popen(
        [str(BIN)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    wait_for_health(f"http://127.0.0.1:{port}")
    return proc


@pytest.fixture(scope="session")
def _binary():
    """Build the debug binary once per test session (after the refactor)."""
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)
    assert BIN.exists(), f"proxy binary missing at {BIN}"
    return BIN


@pytest.fixture(scope="session")
def upstream():
    server, _thread, base = start_upstream_server()
    yield base
    server.shutdown()


@pytest.fixture(scope="session")
def proxy_any(_binary):
    """Proxy with ALLOWED_ORIGINS=* (default)."""
    port = free_port()
    proc = start_proxy(port, "*")
    yield f"http://127.0.0.1:{port}"
    proc.terminate()
    proc.wait(timeout=10)


@pytest.fixture(scope="session")
def proxy_restricted(_binary):
    """Proxy with an explicit allow-list for origin tests."""
    port = free_port()
    proc = start_proxy(port, "https://allowed.com")
    yield f"http://127.0.0.1:{port}"
    proc.terminate()
    proc.wait(timeout=10)


@pytest.fixture()
def reset_options_hits():
    with UpstreamHandler.options_hits_lock:
        UpstreamHandler.options_hits = 0
    yield
    with UpstreamHandler.options_hits_lock:
        UpstreamHandler.options_hits = 0


def options_hits() -> int:
    with UpstreamHandler.options_hits_lock:
        return UpstreamHandler.options_hits
