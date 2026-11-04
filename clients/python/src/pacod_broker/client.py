"""The HTTP clients of pacod-broker's public API: `BrokerClient` (blocking) and
`AsyncBrokerClient` (asyncio). Both have the same methods; answers are the broker's JSON as
dicts (docs/api.md, docs/openapi.json)."""

from __future__ import annotations

import json
import os
from dataclasses import asdict, dataclass, field
from typing import Any, AsyncIterator, Iterator, Literal
from urllib.parse import quote

import httpx

from .sse import Parser, to_event

__all__ = [
    "AsyncBrokerClient",
    "BrokerClient",
    "BrokerError",
    "Outcome",
    "PendingApproval",
    "UsageReport",
]

Outcome = Literal["ok", "client_error", "server_error", "timeout", "rate_limited", "refused"]
ClientReportKind = Literal["abuse", "policy_violation", "non_payment", "no_show", "late_cancel", "good_standing"]

_VERSION = "0.1.0"


class BrokerError(Exception):
    """The broker refused, with its problem (`application/problem+json`)."""

    def __init__(self, status: int, problem: dict[str, Any]):
        self.status = status
        self.problem = problem
        self.kind: str = problem.get("type", "")
        self.errors: list[str] = problem.get("errors", []) or []
        detail = problem.get("detail") or problem.get("title") or f"status {status}"
        text = f"{status} {detail}" + "".join(f"\n  - {e}" for e in self.errors)
        super().__init__(text)

    @property
    def not_found(self) -> bool:
        return self.status == 404


class PendingApproval(BrokerError):
    """The announcement waits for an admin's approval (`BROKER_APPROVAL=new_providers`)."""


@dataclass
class UsageReport:
    """How one call went, with the ticket of the result that was called."""

    ticket: str
    outcome: Outcome
    ttft_ms: float | None = None
    ms_per_token: float | None = None
    #: What was charged for one unit of `rate` (per million tokens for token rates).
    price_charged: float | None = None
    rate: str | None = None
    #: When the call was made, RFC 3339; now by default.
    at: str | None = None

    def to_json(self) -> dict[str, Any]:
        return {k: v for k, v in asdict(self).items() if v is not None}


@dataclass
class _Config:
    base: str
    token: str | None = None
    client_name: str | None = None
    timeout: float = 30.0
    headers: dict[str, str] = field(default_factory=dict)

    def __post_init__(self) -> None:
        self.base = self.base.rstrip("/")
        self.headers = {"user-agent": f"pacod-broker-client-python/{_VERSION}"}
        if self.token:
            self.headers["authorization"] = f"Bearer {self.token}"
        elif self.client_name:
            self.headers["x-broker-client"] = self.client_name


def _env_config(base: str | None, token: str | None, client_name: str | None) -> _Config:
    return _Config(
        base=base or os.environ.get("BROKER_URL") or "http://127.0.0.1:8060",
        token=token if token is not None else (os.environ.get("BROKER_TOKEN") or None),
        client_name=client_name if client_name is not None else (os.environ.get("BROKER_CLIENT") or None),
    )


def _read(r: httpx.Response) -> Any:
    if r.status_code == 202:
        raise PendingApproval(202, _problem(r))
    if r.is_success:
        return r.json() if r.content else None
    raise BrokerError(r.status_code, _problem(r))


def _problem(r: httpx.Response) -> dict[str, Any]:
    try:
        p = r.json()
        return p if isinstance(p, dict) else {"detail": str(p)}
    except (json.JSONDecodeError, ValueError):
        return {"detail": r.text[:300]}


def _lease_path(lease_id: str, provider_name: str | None, suffix: str = "") -> str:
    path = f"/v1/leases/{quote(lease_id, safe='')}{suffix}"
    return f"{path}?provider={quote(provider_name, safe='')}" if provider_name else path


def _check_rating(rating: int) -> None:
    if not 1 <= rating <= 5:
        raise ValueError("a rating is 1 to 5")


class BrokerClient:
    """A blocking client of one broker.

    `base`, `token` and `client_name` default to `BROKER_URL` (http://127.0.0.1:8060),
    `BROKER_TOKEN` (a pacod-id token) and `BROKER_CLIENT` (in open mode, the name this client
    gives itself so that its tickets and reports are its own).
    """

    def __init__(
        self,
        base: str | None = None,
        *,
        token: str | None = None,
        client_name: str | None = None,
        timeout: float = 30.0,
        transport: httpx.BaseTransport | None = None,
    ):
        self.config = _env_config(base, token, client_name)
        self._http = httpx.Client(
            base_url=self.config.base, headers=self.config.headers, timeout=timeout, transport=transport
        )

    def close(self) -> None:
        self._http.close()

    def __enter__(self) -> "BrokerClient":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    @property
    def is_open_mode_client(self) -> bool:
        return not self.config.token

    # --- consumers ----------------------------------------------------------------------

    def discover(self, query: dict[str, Any] | None = None, **filters: Any) -> dict[str, Any]:
        """Runs a discovery query (docs/api.md §3.1). Keyword arguments are added to `query`."""
        return _read(self._http.post("/v1/discover", json={**(query or {}), **filters}))

    def discover_all(self, query: dict[str, Any] | None = None, *, max_results: int = 1000) -> list[dict[str, Any]]:
        """Every page of a query, following the cursor."""
        q = dict(query or {})
        hits: list[dict[str, Any]] = []
        while True:
            page = self.discover(q)
            hits.extend(page["hits"])
            if not page.get("next_cursor") or len(hits) >= max_results:
                return hits[:max_results]
            q["cursor"] = page["next_cursor"]

    def resources(self, **params: Any) -> dict[str, Any]:
        """`GET /v1/resources`: kind, q, tag, protocol, capability, region, currency, sort, …"""
        return _read(self._http.get("/v1/resources", params={k: v for k, v in params.items() if v is not None}))

    def resource(self, resource_id: str) -> dict[str, Any]:
        return _read(self._http.get(f"/v1/resources/{quote(resource_id, safe='')}"))

    def provider(self, provider_id: str) -> dict[str, Any]:
        return _read(self._http.get(f"/v1/providers/{quote(provider_id, safe='')}"))

    def watch(self, after: int | None = None) -> Iterator[dict[str, Any]]:
        """Changes as they happen: dicts with a `type` (hello, resync, upserted, removed,
        provider, generation). Ends when the connection does."""
        params = {"after": after} if after is not None else None
        with self._http.stream("GET", "/v1/watch", params=params, timeout=None) as r:
            if not r.is_success:
                r.read()
                _read(r)
            parser = Parser()
            for chunk in r.iter_bytes():
                for event, data in parser.push(chunk):
                    e = to_event(event, data)
                    if e is not None:
                        yield e

    # --- reports ------------------------------------------------------------------------

    def report_usage(self, *reports: UsageReport | dict[str, Any]) -> dict[str, Any]:
        body = [r.to_json() if isinstance(r, UsageReport) else r for r in reports]
        return _read(self._http.post("/v1/reports/usage", json={"reports": body}))

    def rate(self, ticket: str, rating: int, comment: str | None = None) -> dict[str, Any]:
        _check_rating(rating)
        return _read(self._http.post("/v1/reports/feedback", json={"ticket": ticket, "rating": rating, "comment": comment}))

    def me(self) -> dict[str, Any]:
        return _read(self._http.get("/v1/reputation/me"))

    # --- providers ----------------------------------------------------------------------

    def announce(self, announcement: dict[str, Any]) -> dict[str, Any]:
        """Announces once; `Provider` keeps the lease alive."""
        return _read(self._http.put("/v1/announce", json=announcement))

    def heartbeat(self, lease_id: str, state: dict[str, Any] | None = None, provider_name: str | None = None) -> None:
        _read(self._http.post(_lease_path(lease_id, provider_name, "/heartbeat"), json=state or {}))

    def deregister(self, lease_id: str, provider_name: str | None = None) -> None:
        _read(self._http.delete(_lease_path(lease_id, provider_name)))

    def verify_ticket(self, ticket: str) -> dict[str, Any]:
        return _read(self._http.post("/v1/tickets/verify", json={"ticket": ticket}))

    def report_client(self, ticket: str, kind: ClientReportKind, details: str | None = None) -> dict[str, Any]:
        return _read(self._http.post("/v1/reports/client", json={"ticket": ticket, "kind": kind, "details": details}))

    def schema(self) -> dict[str, Any]:
        return _read(self._http.get("/v1/schema"))

    def ready(self) -> bool:
        try:
            return self._http.get("/readyz").is_success
        except httpx.HTTPError:
            return False


class AsyncBrokerClient:
    """The same client for asyncio (FastAPI services, agents)."""

    def __init__(
        self,
        base: str | None = None,
        *,
        token: str | None = None,
        client_name: str | None = None,
        timeout: float = 30.0,
        transport: httpx.AsyncBaseTransport | None = None,
    ):
        self.config = _env_config(base, token, client_name)
        self._http = httpx.AsyncClient(
            base_url=self.config.base, headers=self.config.headers, timeout=timeout, transport=transport
        )

    async def aclose(self) -> None:
        await self._http.aclose()

    async def __aenter__(self) -> "AsyncBrokerClient":
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.aclose()

    @property
    def is_open_mode_client(self) -> bool:
        return not self.config.token

    async def discover(self, query: dict[str, Any] | None = None, **filters: Any) -> dict[str, Any]:
        return _read(await self._http.post("/v1/discover", json={**(query or {}), **filters}))

    async def resources(self, **params: Any) -> dict[str, Any]:
        return _read(await self._http.get("/v1/resources", params={k: v for k, v in params.items() if v is not None}))

    async def resource(self, resource_id: str) -> dict[str, Any]:
        return _read(await self._http.get(f"/v1/resources/{quote(resource_id, safe='')}"))

    async def provider(self, provider_id: str) -> dict[str, Any]:
        return _read(await self._http.get(f"/v1/providers/{quote(provider_id, safe='')}"))

    async def watch(self, after: int | None = None) -> AsyncIterator[dict[str, Any]]:
        params = {"after": after} if after is not None else None
        async with self._http.stream("GET", "/v1/watch", params=params, timeout=None) as r:
            if not r.is_success:
                await r.aread()
                _read(r)
            parser = Parser()
            async for chunk in r.aiter_bytes():
                for event, data in parser.push(chunk):
                    e = to_event(event, data)
                    if e is not None:
                        yield e

    async def report_usage(self, *reports: UsageReport | dict[str, Any]) -> dict[str, Any]:
        body = [r.to_json() if isinstance(r, UsageReport) else r for r in reports]
        return _read(await self._http.post("/v1/reports/usage", json={"reports": body}))

    async def rate(self, ticket: str, rating: int, comment: str | None = None) -> dict[str, Any]:
        _check_rating(rating)
        return _read(
            await self._http.post("/v1/reports/feedback", json={"ticket": ticket, "rating": rating, "comment": comment})
        )

    async def me(self) -> dict[str, Any]:
        return _read(await self._http.get("/v1/reputation/me"))

    async def announce(self, announcement: dict[str, Any]) -> dict[str, Any]:
        return _read(await self._http.put("/v1/announce", json=announcement))

    async def heartbeat(
        self, lease_id: str, state: dict[str, Any] | None = None, provider_name: str | None = None
    ) -> None:
        _read(await self._http.post(_lease_path(lease_id, provider_name, "/heartbeat"), json=state or {}))

    async def deregister(self, lease_id: str, provider_name: str | None = None) -> None:
        _read(await self._http.delete(_lease_path(lease_id, provider_name)))

    async def verify_ticket(self, ticket: str) -> dict[str, Any]:
        return _read(await self._http.post("/v1/tickets/verify", json={"ticket": ticket}))

    async def report_client(self, ticket: str, kind: ClientReportKind, details: str | None = None) -> dict[str, Any]:
        return _read(
            await self._http.post("/v1/reports/client", json={"ticket": ticket, "kind": kind, "details": details})
        )

    async def ready(self) -> bool:
        try:
            return (await self._http.get("/readyz")).is_success
        except httpx.HTTPError:
            return False
