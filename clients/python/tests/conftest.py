"""Starts a real broker for the tests: the binary of this repository (BROKER_BIN, or
target/debug/pacod-broker after `cargo build -p pacod-broker`) on a free local port, open mode."""

from __future__ import annotations

import os
import socket
import subprocess
import time
from pathlib import Path

import httpx
import pytest

ROOT = Path(__file__).resolve().parents[3]


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture(scope="session")
def broker_url():
    binary = Path(os.environ.get("BROKER_BIN", ROOT / "target" / "debug" / "pacod-broker"))
    if not binary.exists():
        pytest.skip(f"no broker binary at {binary}: cargo build -p pacod-broker")
    port = _free_port()
    env = {
        **os.environ,
        "BROKER_LISTEN": f"127.0.0.1:{port}",
        "BROKER_METRICS_LISTEN": "off",
        "BROKER_REBUILD_MS": "0",
        "BROKER_LEASE_TTL_MIN": "3",
        "BROKER_LOG_FORMAT": "text",
        "RUST_LOG": "warn",
    }
    env = {k: v for k, v in env.items() if not k.startswith("BROKER_") or k in {
        "BROKER_LISTEN", "BROKER_METRICS_LISTEN", "BROKER_REBUILD_MS", "BROKER_LEASE_TTL_MIN", "BROKER_LOG_FORMAT"}}
    proc = subprocess.Popen([str(binary)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    url = f"http://127.0.0.1:{port}"
    for _ in range(100):
        try:
            if httpx.get(f"{url}/readyz").is_success:
                break
        except httpx.HTTPError:
            time.sleep(0.05)
    else:
        proc.kill()
        pytest.fail("the broker did not start")
    yield url
    proc.terminate()
    proc.wait(timeout=10)


ANNOUNCEMENT = {
    "ttl_s": 3,
    "provider": {
        "name": "python-client-test",
        "summary": "A provider in the Python client's test",
        "inventory": {"visibility": "public", "software": [{"id": "o", "kind": "serving_engine", "name": "ollama", "version": "0.5.7"}]},
    },
    "resources": [{
        "key": "qwen",
        "kind": "model",
        "name": "qwen-test",
        "summary": "A model for the Python test",
        "interfaces": [{"protocol": "openai-chat", "url": "http://python.test/v1"}],
        "runs_on": {"software": ["o"]},
        "capabilities": {"tools": True},
        "pricing": {"free": True},
        "access": {"visibility": "public"},
    }],
}


def eventually(check, what: str, timeout: float = 8.0):
    end = time.time() + timeout
    while time.time() < end:
        v = check()
        if v:
            return v
        time.sleep(0.05)
    raise AssertionError(f"{what} did not happen")
