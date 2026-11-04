"""A provider that keeps itself announced.

`Provider.start()` announces and keeps the lease alive in a background thread
(`AsyncProvider` in an asyncio task):

- a heartbeat every `heartbeat_every_s` the broker asked for, with the live state set through
  `set_status`, `set_in_use`, `set_ttft_ms` and `set_resource`;
- when the broker no longer knows the lease (it restarted, or the lease ran out while the
  network was down), the provider announces again by itself;
- when the broker cannot be reached, it tries again with back-off;
- `stop()` says goodbye, which counts in the provider's favour in reputation.
"""

from __future__ import annotations

import asyncio
import copy
import threading
from pathlib import Path
from typing import Any, Callable

import httpx
import yaml

from .client import AsyncBrokerClient, BrokerClient, BrokerError, PendingApproval

__all__ = ["AsyncProvider", "Provider", "load_announcement"]

EventHandler = Callable[[str, Any], None]
"""Called with ("announced", answer), ("reannounced", answer), ("retrying", reason) or ("pending", None)."""


def load_announcement(path: str | Path) -> dict[str, Any]:
    """Reads an announcement from a YAML or JSON file."""
    with open(path, encoding="utf-8") as f:
        doc = yaml.safe_load(f)
    if not isinstance(doc, dict) or "provider" not in doc:
        raise ValueError(f"{path}: an announcement has `provider` and `resources`")
    return doc


class _Live:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._all: dict[str, Any] = {}
        self._per_key: dict[str, dict[str, Any]] = {}

    def set(self, **values: Any) -> None:
        with self._lock:
            self._all.update({k: v for k, v in values.items() if v is not None})

    def set_resource(self, key: str, **values: Any) -> None:
        with self._lock:
            self._per_key.setdefault(key, {}).update({k: v for k, v in values.items() if v is not None})

    def heartbeat(self) -> dict[str, Any]:
        with self._lock:
            hb = dict(self._all)
            if self._per_key:
                hb["resources"] = copy.deepcopy(self._per_key)
            return hb


def _wait(every: int, failures: int) -> float:
    # After a failure, try again sooner, but never more often than every second.
    return float(every) if failures == 0 else float(min(2 ** min(failures, 5), every))


class _LiveMixin:
    _live: _Live

    def set_status(self, status: str) -> None:
        """active, degraded, draining or maintenance."""
        self._live.set(status=status)

    def set_in_use(self, n: int) -> None:
        self._live.set(in_use=n)

    def set_ttft_ms(self, ms: float) -> None:
        self._live.set(ttft_ms=ms)

    def set_resource(
        self, key: str, *, status: str | None = None, in_use: int | None = None, ttft_ms: float | None = None
    ) -> None:
        """The live state of one resource, by its key, over the provider-wide one."""
        self._live.set_resource(key, status=status, in_use=in_use, ttft_ms=ttft_ms)


class Provider(_LiveMixin):
    """Announces `announcement` and keeps it alive until `stop()`. Also a context manager."""

    def __init__(
        self, client: BrokerClient, announcement: dict[str, Any], on_event: EventHandler | None = None
    ):
        self.client = client
        self.announcement = announcement
        self.on_event = on_event or (lambda kind, value: None)
        self.announced: dict[str, Any] | None = None
        self._live = _Live()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        # Open-mode brokers know the lease's owner by the provider's name.
        self._owner = announcement["provider"]["name"] if client.is_open_mode_client else None

    def start(self) -> dict[str, Any]:
        self.announced = self.client.announce(self.announcement)
        self.on_event("announced", self.announced)
        self._thread = threading.Thread(target=self._run, name="pacod-broker-heartbeat", daemon=True)
        self._thread.start()
        return self.announced

    def _run(self) -> None:
        failures = 0
        while True:
            assert self.announced is not None
            if self._stop.wait(_wait(self.announced["heartbeat_every_s"], failures)):
                return
            try:
                self.client.heartbeat(self.announced["lease_id"], self._live.heartbeat(), self._owner)
                failures = 0
            except BrokerError as e:
                if not e.not_found:
                    failures += 1
                    self.on_event("retrying", str(e))
                    continue
                try:
                    self.announced = self.client.announce(self.announcement)
                    failures = 0
                    self.on_event("reannounced", self.announced)
                except PendingApproval:
                    failures = 5
                    self.on_event("pending", None)
                except (BrokerError, httpx.HTTPError) as e2:
                    failures += 1
                    self.on_event("retrying", str(e2))
            except httpx.HTTPError as e:
                failures += 1
                self.on_event("retrying", str(e))

    def stop(self) -> None:
        """Stops the heartbeats and ends the lease cleanly."""
        self._stop.set()
        if self._thread:
            self._thread.join(timeout=10)
        if self.announced:
            try:
                self.client.deregister(self.announced["lease_id"], self._owner)
            except BrokerError as e:
                if not e.not_found:
                    raise

    def abandon(self) -> None:
        """Stops the heartbeats without a goodbye, as a crash would: the lease runs out after its
        TTL and counts against the provider's lease discipline."""
        self._stop.set()
        if self._thread:
            self._thread.join(timeout=10)

    def __enter__(self) -> "Provider":
        self.start()
        return self

    def __exit__(self, *exc: object) -> None:
        self.stop()


class AsyncProvider(_LiveMixin):
    """The same for asyncio: `await start()`, `await stop()`, or `async with`."""

    def __init__(
        self, client: AsyncBrokerClient, announcement: dict[str, Any], on_event: EventHandler | None = None
    ):
        self.client = client
        self.announcement = announcement
        self.on_event = on_event or (lambda kind, value: None)
        self.announced: dict[str, Any] | None = None
        self._live = _Live()
        self._task: asyncio.Task[None] | None = None
        self._owner = announcement["provider"]["name"] if client.is_open_mode_client else None

    async def start(self) -> dict[str, Any]:
        self.announced = await self.client.announce(self.announcement)
        self.on_event("announced", self.announced)
        self._task = asyncio.create_task(self._run(), name="pacod-broker-heartbeat")
        return self.announced

    async def _run(self) -> None:
        failures = 0
        while True:
            assert self.announced is not None
            await asyncio.sleep(_wait(self.announced["heartbeat_every_s"], failures))
            try:
                await self.client.heartbeat(self.announced["lease_id"], self._live.heartbeat(), self._owner)
                failures = 0
            except BrokerError as e:
                if not e.not_found:
                    failures += 1
                    self.on_event("retrying", str(e))
                    continue
                try:
                    self.announced = await self.client.announce(self.announcement)
                    failures = 0
                    self.on_event("reannounced", self.announced)
                except PendingApproval:
                    failures = 5
                    self.on_event("pending", None)
                except (BrokerError, httpx.HTTPError) as e2:
                    failures += 1
                    self.on_event("retrying", str(e2))
            except httpx.HTTPError as e:
                failures += 1
                self.on_event("retrying", str(e))

    async def stop(self) -> None:
        if self._task:
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
        if self.announced:
            try:
                await self.client.deregister(self.announced["lease_id"], self._owner)
            except BrokerError as e:
                if not e.not_found:
                    raise

    async def abandon(self) -> None:
        """Stops the heartbeats without a goodbye, as a crash would."""
        if self._task:
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass

    async def __aenter__(self) -> "AsyncProvider":
        await self.start()
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.stop()
