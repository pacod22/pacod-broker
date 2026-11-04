"""A small server-sent-events reader for the watch stream."""

from __future__ import annotations

import json
from typing import Any

KNOWN = {"hello", "resync", "upserted", "removed", "provider", "generation"}


class Parser:
    """Feed bytes; get the (event, data) pairs they complete. Comments (keep-alives) are skipped."""

    def __init__(self) -> None:
        self._buf = b""
        self._event = ""
        self._data: list[str] = []

    def push(self, chunk: bytes) -> list[tuple[str, str]]:
        self._buf += chunk
        out: list[tuple[str, str]] = []
        while b"\n" in self._buf:
            raw, self._buf = self._buf.split(b"\n", 1)
            line = raw.decode("utf-8", "replace").rstrip("\r")
            if line == "":
                if self._event or self._data:
                    out.append((self._event, "\n".join(self._data)))
                self._event, self._data = "", []
            elif line.startswith("event:"):
                self._event = line[6:].lstrip()
            elif line.startswith("data:"):
                v = line[5:]
                self._data.append(v[1:] if v.startswith(" ") else v)
        return out


def to_event(event: str, data: str) -> dict[str, Any] | None:
    """The watch event as a dict with `type`; None for kinds this client does not know."""
    if event not in KNOWN:
        return None
    try:
        body = json.loads(data) if data else {}
    except json.JSONDecodeError:
        body = {}
    if not isinstance(body, dict):
        body = {}
    body["type"] = event
    return body
