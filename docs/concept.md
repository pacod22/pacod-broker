# pacod-broker: concept (design v1.0)

| Field | Value |
|---|---|
| Status | Built: the first release |
| Origin | The capability broker of *AI Broker & GW*: pacod-gw's `docs/ai-gateway-feature-catalog.md`, section 12 (BRK-01 to BRK-13), with the critique of its `docs/ai-gateway-design-assessment.md`, section 4.10 |
| Decisions | Soft state with leases, plus an optional state directory. One node in this release, with ids ready for federation. Access through pacod-id tokens, with an open mode for development. The console is its own app in this repository. |

---

## 1. What it is

A broker where **providers announce** what they offer and **consumers discover** it. What can be announced:

- AI models;
- MCP servers;
- agents;
- tools;
- compute such as GPU hours;
- embeddings;
- datasets.

Each resource comes with its interfaces, capabilities, prices over time, availability over time, data policy and, optionally, the hardware and software it runs on. Both sides have a reputation built from evidence ([reputation.md](reputation.md)).

Three requirements shaped it:

1. **Fast and small.** One static binary with no database and no external dependency at run time. Lookups take no lock.
2. **Runs without administration.** Every setting has a default. Providers keep their own entries alive with leases, and what they stop renewing disappears on its own.
3. **Manageable when wanted.** An admin API on its own listener, and a console that docks to it, reusing pacod-id for sign-in, roles and design.

## 2. Architecture

```text
            providers                                 consumers, agents, gateways
   PUT /v1/announce, heartbeats           POST /v1/discover · GET /v1/resources · /mcp · reports
                 │                                              │
                 ▼                                              ▼
 ┌─────────────────────────────── pacod-broker (one process) ────────────────────────────────┐
 │  public listener :8060 ─────────────┐                    admin listener :8061 (optional)  │
 │                                     ▼                                │                    │
 │  registry ── records under one lock ──► rebuild (coalesced) ──► ArcSwap<Index> ◄── queries│
 │     │  leases ──► Live cells (atomics) ◄── heartbeats              (no lock)             │
 │     │  sweeper (1 s): expired leases; uptime samples (1 min)                               │
 │     ▼                                                                                      │
 │  reputation engine ── evidence ──► decayed aggregates ──► scores (ArcSwap, every 60 s)     │
 │  tickets (HMAC) · pacod-id (feed, keys, introspection) · /metrics :9160                    │
 └───────────────────────────────── BROKER_STATE_DIR (optional) ───────────────────────────────┘
                     static.json · decisions.json · ticket.key · reputation.log/.snap
```

| Part | How |
|---|---|
| Index | An immutable generation holding posting lists for: kind, tag, protocol, capability, region, provider, GPU model and software name. It also has a BM25 text index over names, titles, summaries, descriptions and tags, and name ranks. Queries load it with one atomic read. |
| Live state | Each resource has a cell of atomics: lease expiry, status, capacity in use and observed time to first token. Heartbeats write to the cell and never rebuild the index. |
| Rebuilds | A write (announce, end of a lease, an admin action) marks the index dirty. One task gathers changes for `BROKER_REBUILD_MS` (20 ms), builds the next generation off the runtime's threads and swaps it in. The generation number is the cursor of the watch stream. |
| Leases | A lease is per provider session, not per resource: a provider with a thousand models sends one small heartbeat. |
| Time | Prices and availability are evaluated per candidate, at the instant or over the span asked. Each query converts its instant to each time zone once (`schedule::Clock`). Recurring rules are expanded only for the days a span covers, and a span may be at most 62 days. |
| Queries | Two passes. The first runs cheap filters and the sort key without allocating. The second builds full hits (effective price, live view, reasons, ticket) for the page returned only. |
| Persistence | Leased entries are never written: providers announce again after a restart. Static entries come from the catalogue file (`BROKER_CATALOG_FILE`, YAML, reread on SIGHUP) and the admin API (kept in `BROKER_STATE_DIR`). |
| Federation | Ids are derived from who announces (`prv_` from the broker id and the principal, `res_` from the provider and key), and each record carries its `origin`. The data needs no change for brokers to exchange it later. |

**Measured** at 10 000 resources, release build (`make bench`; crates/pacod-broker-index/benches/discover.rs):

| Query | Time |
|---|---|
| Capabilities and protocol | 0.23 ms |
| Over a time span with coverage | 0.54 ms |
| Hardware and software | 1.2 ms |
| Kind with a price sort | 1.8 ms |
| A text that matches every entry | 3.4 ms |
| The whole catalogue, no filter | 2.7 ms |
| An index rebuild | 150 ms |

Other measurements, with the example catalogue loaded:

| Measure | Value |
|---|---|
| Release binary | 12.7 MB |
| Start-up | 56 ms |
| Idle memory (RSS) | 23 MB |

The target was a p99 under 1 ms. Filtered queries meet it or come close; queries that touch every entry do not yet. Section 9 lists the next steps.

## 3. Data model

### 3.1 Provider

| Field | What |
|---|---|
| `name` | A slug. In open mode it also identifies the provider. |
| `title`, `summary`, `description`, `website`, `contact`, `terms_url`, `privacy_url`, `i18n` | Human-readable: a one-line summary, Markdown description, translations |
| `locations`, `certifications` | Where it operates; certifications it declares |
| `inventory` | Optional, with its own `visibility`: `hardware[]` (`id`, `type` gpu/cpu/tpu/npu/accelerator/memory/storage/network, `vendor`, `model`, `count`, `memory_gb`, `interconnect`, `region`) and `software[]` (`id`, `kind` os/driver/runtime/framework/library/serving_engine/model_format, `name`, `version`). Everything is marked `declared`. |

The broker adds `verified` (from pacod-id or an admin), `organisation_id`, `source` (lease or static) and the reputation.

### 3.2 Resource

One resource is one endpoint: the same model at two providers is two resources (BRK-01, as in pacod-gw's model repository).

| Field | What |
|---|---|
| `key`, `kind`, `name`, `version`, `tags` | Identity. `kind` is one of model, mcp_server, agent, tool, compute, embedding, dataset, or `other:<slug>`. |
| `title`, `summary`, `description`, `documentation_url`, `examples`, `i18n` | Human-readable. The summary is one line of at most 200 characters; the description is Markdown, at most 16 KiB by default. |
| `interfaces[]` | `protocol`, `url`, `auth` (`scheme`, `obtain_url`), `spec_url`, `version`. Known protocols: openai-chat, openai-responses, anthropic-messages, gemini, pacod-gw, mcp-streamable-http, mcp-sse, a2a, openapi, grpc, http. |
| `model` | `family`, `upstream_id`, `context_window`, `max_output_tokens`, `modalities`, `quantisation`, `parameters_b` |
| `capabilities` | Flags (tools, vision, reasoning, structured_output, streaming, prompt_cache, embeddings), MCP tools, compute facts, and `extra` |
| `runs_on` | The inventory items it runs on, by id |
| `pricing` | Section 3.3 |
| `availability` | Section 3.4 |
| `policy` | `data_retention`, `training_use`, `residency`, `classification` |
| `access` | `visibility` (public, platform, `organisation:<id>`), `min_client_reputation`, `allow_new_clients` |

### 3.3 Prices over time

```text
pricing { currency, free, timezone,
  periods[] { valid_from, valid_to?, rates,           # versions; never retroactive
              windows[]   { name, weekdays, from_time, to_time, rates }   # off-peak, weekends
              overrides[] { from, to, rates, reason } # one-off: a promotion, spot price
              tiers[]     { above, unit, rates }      # volume
              commitments[] { min_hours, rates } } }  # reserved
```

- **Rates.** Per million tokens: input, output, cache_read, cache_write, reasoning. Per unit: per_request, per_call, per_image, per_audio_second, per_second, per_hour, per_gpu_hour.
- **Resolution at an instant:**
  1. the period valid then;
  2. an override covering the instant, or else the first window open then, or else the base rates;
  3. a rate the variant leaves out keeps its base value.
- **Over a span:** the lowest, highest and time-weighted mean of each rate, and the instants at which the source changes.
- **Never retroactive.** A period that has started cannot change. It can be ended, not before now, and followed by a new one.
- **No currency conversion.** Discovery's `currency` filter keeps comparisons within one currency.

### 3.4 Availability over time

```text
availability { valid_from?, valid_to?, status, regions, capacity { max_concurrency, unit },
  latency { ttft_ms, ms_per_token },
  schedule { timezone,
    rules[]       { weekdays, from_time, to_time, state: available|unavailable, capacity? },
    maintenance[] { from, to, reason },                          # the broker stamps announced_at
    slots[]       { from, to, capacity, state: available|reserved|tentative } } }
```

The capacity at an instant is decided in this order:

1. zero outside `valid_from`/`valid_to`, and zero inside maintenance;
2. a slot covering the instant decides, when there is one;
3. otherwise the rules: when there are rules, a time that no rule opens is closed;
4. otherwise the declared maximum.

The live status and the capacity in use come with heartbeats. They count only within 15 minutes of now: a provider draining now says nothing about next week.

## 4. Discovery (BRK-06)

`POST /v1/discover` takes these filters:

- **What it is:** `kind`, `q` (words, prefixes from three letters), `tags`, `protocols`, `capabilities`, `provider`, `model` (`family`, `min_context_window`, `min_output_tokens`, input and output modalities, `quantisation`).
- **Where data goes:** `regions`, `residency` (every place the resource lists must be allowed), `no_training_use`.
- **What it runs on:** `hardware` (`gpu_model`, `min_gpu_memory_gb`, `min_gpus`, `type`, `min_memory_gb`) and `software` (`name`, `version` as a semver requirement). These match only an inventory the caller may see.
- **Price:** `max_price` per rate, `currency`, `include_unpriced`.
- **Time and capacity:** `at` or `during {from, to}`, `availability` (`min_capacity`, `coverage` full or any, `exclude_degraded`), `include_unavailable`.
- **Reputation:** `min_reputation`, `min_confidence`.

**Sort.** `relevance` (with words) or `name` by default. `price:<rate>`, `ttft` and `reputation` need a `kind`: they compare only within one kind. Ties break by id, so the same query gives the same order.

**Each result** carries:
- the resource and its provider;
- the effective price (at the instant) or a price summary (over the span);
- availability and capacity;
- the live state;
- reputations;
- `why`: the filters it passed, the sort key and its value;
- a `ticket`.

Pages are 20 results by default and 200 at most, with a cursor.

The same engine answers `GET /v1/resources` (common filters as parameters) and the MCP tool `discover_resources`.

**Visibility.** `public` is seen by everyone signed in, and by anonymous callers when `BROKER_PUBLIC_DISCOVERY` is true. `platform` is seen by everyone signed in. `organisation:<id>` is seen by members of that organisation. In open mode everyone sees every visibility. Providers' `min_client_reputation` still applies to a client that names itself; only broker admins pass it.

## 5. Access (pacod-id)

`BROKER_AUTH=open` is the default and is meant for development and closed networks: anyone may do anything. A client may name itself with `X-Broker-Client` so that tickets and reports can be tried.

`BROKER_AUTH=oidc` uses pacod-id. It runs on pacod-id's own client library, vendored at the commit pacod-gw pins:

| Concern | How |
|---|---|
| People | Access tokens checked locally against pacod-id's keys (audience `pacod-broker`). |
| Providers and machines | **Service accounts** with `pidt_…` tokens, introspected and cached for 60 s. Revocation reaches the broker through the change feed. They are created in pacod-id's administration (`/admin/service-accounts`). |
| Roles | `authz/broker.yaml`, pushed to pacod-id at every start. Roles: `admin`, `operator`, `moderator`, `viewer` (platform); `provider` and `consumer` (platform or organisation); `witness` (platform: gateways that report usage on the calls they carried); `provisioner` (platform: a product such as pacod-cms that makes a provider account in each of its organisations, holding `members.manage` only); `org_admin` (organisation); and `member`, implicit for every person (discover and rate). The broker follows pacod-id's feed of role bindings and decides locally. |
| pacod-id unavailable | Discovery never waits for pacod-id. While the copy of the bindings is not ready, callers are anonymous, and the admin API answers 503 `identity_unavailable`. |

## 6. The console

`ui/` is a SvelteKit 3 app. It follows pacod-gw's console in three ways:

- it uses pacod-id's design package (`@pacod/ui` tokens and console styles);
- people sign in through `@pacod/id-sveltekit`, as a confidential client with a sealed cookie;
- it calls a typed client generated from [openapi.json](openapi.json).

**Docking.** It holds no state and docks to the admin API of the brokers that `BROKER_CONSOLE_BROKERS` lists, with a switcher when there are several. The browser never talks to a broker.

**Pages:**
- overview;
- resources, and a resource page with its description, a seven-day availability and price timeline, interfaces, price periods and reputation;
- providers, and a provider page with its profile, inventory, leases and actions;
- reputation, and moderation;
- static entries and leases;
- metrics, from the shared Prometheus;
- access: pacod-id roles and service accounts;
- settings.

## 7. Monitoring and logging

- **Metrics.** `/metrics` on `BROKER_METRICS_LISTEN` (job `pacod-broker`, [platform/prometheus/broker.yml](../platform/prometheus/broker.yml)). No label holds an id.
- **Dashboard and alerts.** [deploy/grafana/dashboards/broker-overview.json](../deploy/grafana/dashboards/broker-overview.json) and [deploy/prometheus/rules/broker.rules.yml](../deploy/prometheus/rules/broker.rules.yml).
- **Logs.** In the pacod format (pacod-demo/docs/LOGGING.md), through the logging crate copied from pacod-gw. Requests are logged as `<METHOD> <path> <status>`, with health checks at debug.

## 8. The catalogue's broker features, placed

| Id | Feature | Here |
|---|---|---|
| BRK-01 | Resource model per endpoint | Built (section 3) |
| BRK-02 | Registration paths | Configuration in code (the catalogue file), the admin API, and self-registration with a heartbeat TTL are built. Signing is done by pacod-id tokens. A Kubernetes CRD is not built. |
| BRK-03 | Life cycle | Lease-based: announced, live (active, degraded, draining, maintenance), gone. Approval of new providers is optional (`BROKER_APPROVAL`). Validation is synchronous. |
| BRK-04 | Health and metrics | Passive: heartbeats with live state; usage reports with latency and outcome. Active probes are deferred. |
| BRK-05 | Cost model | Built: periods, windows, overrides, tiers, commitments, never retroactive. No traffic-derived local rate (assessment 4.10). |
| BRK-06 | Discovery and ranking | Hard filters and sort within a kind, with reasons. Weighted pick and soft thresholds are deferred. |
| BRK-07 | Quality signals | Ratings and observed latency and reliability. Canaries and evaluations are deferred. |
| BRK-08 | Signed bindings | Deferred. The interaction ticket is a first, smaller step. |
| BRK-09 | Exposure | REST, SSE watch and MCP tools are built. gRPC is not built. |
| BRK-10 | Visibility | public, platform and organisation, plus client-reputation gating |
| BRK-11 | Capacity | Declared, plus in-use from heartbeats; slots. vLLM scraping is deferred. |
| BRK-12 | Catalogue and approval | Approval of new providers, verification and moderation are built. Semantic search is deferred (BM25 for now). |
| BRK-13 | Import mappings | Deferred: `/v1/models`, MCP `tools/list`, A2A Agent Card and OpenAPI into a draft announcement |

**The assessment's points (4.10), and how they are met:**
- Ranking happens only within a kind.
- There is no traffic-derived cost.
- Latency is split into TTFT and per-token latency.
- Every result returns its reasons.
- Callers that pin a model pin it: the broker never picks for them.

## 9. Next

1. **Index speed.**
   - Precompute each entry's availability shape and price variants for the query instant.
   - Rebuild the text index incrementally.
   - Bring whole-catalogue queries under 1 ms.
2. **pacod-gw (BRK-13).** The gateway as provider, as a source for its model repository and as a witness: the concept is [pacod-gw.md](pacod-gw.md).
3. **Slot booking.** It would make `commitment` evidence first-hand.
4. **Federation.** Brokers exchanging records and evidence by `origin`.
5. **A Helm chart.**
