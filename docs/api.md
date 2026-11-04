# pacod-broker: API

The complete, generated description is [openapi.json](openapi.json) (OpenAPI 3.1). `pacod-broker openapi` prints it, and `pacod-broker schema` prints the JSON Schema of an announcement. This page shows how the parts fit, with examples. Errors are `application/problem+json`, each with a stable `type`; validation problems list every one at once in `errors`.

## 1. Listeners

| Listener | Default | What |
|---|---|---|
| Public, `BROKER_LISTEN` | `0.0.0.0:8060` | Everything below except section 5 |
| Admin, `BROKER_ADMIN_LISTEN` | off | `/api/v1/…`, section 5 |
| Metrics, `BROKER_METRICS_LISTEN` | `0.0.0.0:9160` | `/metrics` |

**Authentication.**
- With `BROKER_AUTH=oidc`: `Authorization: Bearer <pacod-id access token or pidt_… service-account token>`.
- In open mode no token is needed. `X-Broker-Client: <name>` identifies a client for tickets and reports.

## 2. Providers

### 2.1 Announce

```http
PUT /v1/announce
{
  "ttl_s": 30,
  "provider": { "name": "sofia-gpu", "summary": "…", "inventory": { … } },
  "resources": [ { "key": "llama-70b", "kind": "model", "name": "llama-3.3-70b-instruct", … } ]
}
```

```json
{ "lease_id": "lse_…", "provider_id": "prv_…", "ttl_s": 30, "heartbeat_every_s": 10,
  "expires_at": "…", "resources": { "llama-70b": "res_…" }, "warnings": [] }
```

- **Replacement.** An announcement replaces the lease's set. To keep the same lease after a provider restarts, send its `lease_id`. Without it, a new lease opens.
- **Stable ids.** Resource ids are derived from the provider and the key, so they stay the same across announcements and restarts.
- **Approval.** With `BROKER_APPROVAL=new_providers`, a provider's first announcement answers 202 `pending_approval`.
- **Field reference.** [examples/catalog.example.yaml](../examples/catalog.example.yaml) shows every field; [concept.md](concept.md) section 3 explains them.

### 2.2 Heartbeat and goodbye

```http
POST /v1/leases/{lease_id}/heartbeat
{ "status": "active", "in_use": 3, "ttft_ms": 280, "resources": { "llama-70b": { "status": "degraded" } } }
```

- **Body.** Everything in the body is optional. Top-level fields apply to every resource of the lease; `resources` overrides them per key.
- **Unknown lease.** 404 means the lease is unknown: announce again.
- **Goodbye.** `DELETE /v1/leases/{lease_id}` ends a lease cleanly. Reputation counts this in the provider's favour.
- **Open mode.** Add `?provider=<name>` to both calls.

## 3. Consumers

### 3.1 Discover

```http
POST /v1/discover
{
  "kind": "model",
  "capabilities": ["tools"],
  "model": { "min_context_window": 100000 },
  "currency": "EUR",
  "max_price": { "input": 0.5 },
  "during": { "from": "2026-11-10T20:00:00Z", "to": "2026-11-11T06:00:00Z" },
  "availability": { "min_capacity": 4, "coverage": "full" },
  "hardware": { "gpu_model": "H100-SXM" },
  "software": [{ "name": "vllm", "version": ">=0.7" }],
  "min_reputation": 0.6,
  "sort": "price:input",
  "limit": 10
}
```

Each hit has these fields:

| Field | What |
|---|---|
| `resource`, `provider` | The resource and a summary of its provider |
| `price` or `price_summary` | The effective price at the instant, or lowest, highest and mean over the span |
| `available`, `capacity`, `availability_summary` | Availability at the instant or over the span |
| `live` | Status, capacity in use, observed latency, lease expiry |
| `reputation`, `provider_reputation` | Scores with their components |
| `why` | The filters passed, the sort key and its value |
| `ticket` | Proof of the interaction, to quote in reports |

`concept.md` section 4 lists every filter. Sorting by price, latency or reputation needs `kind`.

### 3.2 The simple form and single reads

| Request | What |
|---|---|
| `GET /v1/resources?kind=model&q=llama&tag=llm,open-weights&protocol=openai-chat&currency=EUR&sort=name` | The same engine; lists are comma-separated |
| `GET /v1/resources/{id}` | One resource with its price and availability now, and a ticket |
| `GET /v1/providers/{id}` | A profile, its inventory if visible, reputation and resources |
| `GET /v1/schema` | The JSON Schemas of what the API takes |

### 3.3 Watch

`GET /v1/watch?after=<generation>` is a stream of server-sent events, for gateways and consoles that keep a copy.

| Event | When |
|---|---|
| `hello` (or `resync`) | First, with the current generation. `resync` when the caller's `after` is behind it. |
| `upserted`, `removed` | A resource was announced or updated; a resource went |
| `provider` | Blocked, approved, verified, pending |
| `generation` | A new index generation is in place |
| `resync` | The stream fell behind: read the catalogue again |

## 4. Reputation

| Request | Body |
|---|---|
| `POST /v1/reports/usage` | `{"reports": [{"ticket", "outcome": "ok\|client_error\|server_error\|timeout\|rate_limited\|refused", "ttft_ms", "ms_per_token", "price_charged", "rate", "at"}]}`, up to 1000 |
| `POST /v1/reports/feedback` | `{"ticket", "rating": 1–5, "comment"}` |
| `POST /v1/reports/client` | (providers) `{"ticket", "kind": "abuse\|policy_violation\|non_payment\|no_show\|late_cancel\|good_standing", "details"}` |
| `POST /v1/tickets/verify` | (providers) `{"ticket"}` returns the client, the resource and the client's reputation |
| `GET /v1/reputation/me` | The caller's own scores |

Answers say how many reports counted, and why the others did not. See [reputation.md](reputation.md).

## 5. Admin API (`/api/v1`, admin listener)

| Request | Needs | What |
|---|---|---|
| `GET /session` | broker.read | Who the caller is; the console asks once per request |
| `GET /status`, `GET /config` | broker.read | State and effective configuration, without secrets |
| `GET /resources`, `/resources/{id}`, `/resources/{id}/timeline` | broker.read | Every resource, available or not; one with its lease and evidence; prices and availability over a span (7 days by default) |
| `GET /providers`, `/providers/{id}` | broker.read | With leases, blocked flag, evidence |
| `POST /providers/{id}/block`, `/unblock`, `/approve` | broker.manage | Blocking ends the provider's leases and refuses its announcements |
| `POST /providers/{id}/verify` `{"verified"}` | broker.manage | Vouch for a provider (reputation `trust`) |
| `POST /providers/{id}/incident` `{"note"}` | reputation.moderate | An upheld incident; costs trust |
| `GET /pending`, `GET /leases`, `DELETE /leases/{id}` | broker.read / manage | |
| `GET /static`, `PUT /static`, `DELETE /static/{provider_id}` | broker.read / manage | Static entries, kept in `BROKER_STATE_DIR` |
| `GET /reputation?side=&sort=`, `/reputation/{side}/{id}` | broker.read | Scores, lowest first; one score with its evidence |
| `POST /reputation/{side}/{id}/freeze\|unfreeze\|reset` | reputation.moderate | |
| `GET /reports`, `POST /reports/{id}/reject` | broker.read / moderate | Client reports and notes; take back an unfounded report |
| `GET /events` | broker.read | Changes as server-sent events |
| `GET /openapi.json` | none | This document |

In open mode, `BROKER_ADMIN_TOKEN`, when set, is required as the bearer token.

## 6. MCP

`POST /mcp` speaks MCP over streamable HTTP and answers with JSON. It implements `initialize`, `ping`, `tools/list` and `tools/call`. The tools are:

| Tool | Arguments |
|---|---|
| `discover_resources` | The discovery query of section 3.1 |
| `describe_resource` | `{"id"}` |
| `describe_provider` | `{"id"}` |

Results come as text and as `structuredContent`.

```bash
examples/mcp.sh
```
