# pacod-broker and pacod-gw: an integration concept

pacod-gw is the gateway every LLM call of the platform goes through. pacod-broker is the catalogue of what can be called, at what price, when, and how well it serves. This page is the concept for using the two together: the roles the gateway can take towards the broker, how each would work, scenarios that combine them, and a phased recommendation.

**Status.** Built, all three phases: pacod-gw's branch `broker-integration` and pacod-demo's branch of the same name, with this repository's price feed and delegated witness. Section 11 says what was built and where it differs from the concept; the sections before it are the concept as written. Names from pacod-gw refer to its code at the time of writing (`crates/pacod-gw-core`, `crates/pacod-gw-events`, `crates/pacod-gw-analytics`, `ui/`).

**Origin.** pacod-gw's feature catalogue §12 (BRK-01 to BRK-13) asks for a capability broker. Its design assessment §4.10 calls the broker "elegant, value unproven", because "apps pin models". The concept therefore starts where the value is clear and the risk is low, and grows only as demand shows.

## 1. Principles

Both products already hold these. The integration keeps every one.

| Principle | Where it comes from |
|---|---|
| **Pinned stays pinned.** The broker never picks a model for a caller, and the gateway never swaps one silently. | gw: "the gateway never balances or falls back on its own"; assessment §4.10, point 1; broker concept §8 |
| **Nothing in the gateway changes by itself.** Broker data reaches gw as proposals an admin accepts, or as prices within the price tracker's bounds. | gw model discovery and price tracker: "Nothing is activated or changed automatically" |
| **The broker is never on the gateway's request path.** A call through gw never waits for the broker; gw works the same with the broker down. | broker rule: discovery never waits for pacod-id; gw rule: the data plane reads only its in-memory snapshot |
| **No credentials in the broker.** An interface names its auth scheme and where to obtain access (`auth.obtain_url`), never a secret. | broker data model |
| **No currency conversion in the broker.** gw converts with its exchange rates; the broker shows each price in its own currency. | broker rule |
| **Rank only within a kind; evidence needs a ticket.** A provider's own organisation cannot raise its reputation. | broker rules; assessment §4.10, point 2 |

## 2. What meets what

| pacod-gw today | pacod-broker today |
|---|---|
| **Model repository** in `config_objects`: `Model` (one provider endpoint for one upstream model), `Price` (`rates`, `variants`, `tiers`, `windows` in UTC, `effective_from`), `ModelState` (draft, active, deprecated, retired, disabled) | **Resources**: `ResourceSpec` with `kind`, interfaces, `pricing` (periods, zone-aware windows, overrides, tiers), availability over time, live state |
| **Model discovery** (`mgmt/discovery.rs`): providers' model lists become `discovery_proposals` (`new`, `gone`, `update`) | `POST /v1/discover`, the watch stream (`/v1/watch`), `GET /v1/resources/{id}` |
| **Price tracker** (`model/pricejob.rs`): feeds in format `pacod` or `litellm`, pages; applied within a bound, else proposed | Prices in force at any instant, never retroactive |
| **Templates and pools** (`TemplateSpec`, `PoolEntry`, up to 16), `RoutingScheme`, algorithms including `cheapest` and `lowest_latency`; a breaker per credential and model (`routing/live.rs`, `RouteRegistry`, `TargetState`) | Reputation per resource, provider and client; heartbeats; leases that end when a provider stops |
| **One `UsageEvent` per call** (`pacod-gw-events/src/usage.rs`): status, `upstream_first_byte_ms`, `upstream_stream_ms`, tokens, `cost`; spooled to gw-analytics | `POST /v1/reports/usage` (`outcome`, `ttft_ms`, `ms_per_token`, `price_charged`), with an interaction ticket |
| **pacod-id**: service accounts, `authz/gw.yaml` | **pacod-id**: roles `broker.provider`, `broker.consumer`, `authz/broker.yaml` |
| **Siblings dock** by pushing through the management API (pacod-benchmark: `POST /api/v1/benchmark-reports`) and by an optional URL in the console (`TESTER_URL`) | Clients in Rust, Python and TypeScript ([clients.md](clients.md)); MCP at `/mcp` with `discover_resources` and `describe_resource` |

**Two constraints of the broker shape every option below:**
1. **A usage report counts only from the client the ticket was issued to** (`tickets.rs`). A gateway cannot report on its callers' tickets as things stand.
2. **One client counts at most 120 usage reports per resource per hour** (`USAGE_PER_HOUR`). A gateway with heavy traffic reports samples, not every call.

## 3. Roles

The gateway can take seven roles towards the broker. They are independent; the phases in section 8 combine them.

### 3.1 R1: the gateway as a provider

**What.** An organisation offers what it runs on the gateway to others on the platform: its templates and selected models become broker resources.

**How it works.**
1. An organisation opts in and names what it offers: templates (recommended: stable, versioned, hiding upstream changes) or individual models.
2. A bridge (section 4) announces each offering as a resource:
   - `kind` from the model's kind (section 5);
   - interfaces `pacod-gw` plus the dialects gw serves (`openai-chat`, `anthropic-messages`, `gemini`), each pointing at the gateway's public URL;
   - `auth.scheme: bearer`, `obtain_url` pointing at where a workspace key is requested;
   - prices from the model's active `Price`, or the template's first pool model;
   - visibility `organisation:<id>`, or `platform` when the organisation chooses.
3. The bridge heartbeats with live state (section 5): `degraded` when every credential's breaker is open, `in_use` from calls in flight, `ttft_ms` from the first-byte average.
4. A model that is retired or disabled drops out of the next announcement. Deprecated models stay, with their sunset in the description.
5. Leases are held by one bridge per organisation, never by every gateway instance.

**Attachment.** gw: the management API, read only. Broker: `POST /v1/announce` and heartbeats.

**Risks.** Prices drift between gw and the broker if announcements lag; the bridge re-announces on gw's config events (`GET /api/v1/events`). Visibility mistakes expose offerings; the default is the organisation's own.

### 3.2 R1b: the gateway as the front door

**What.** A consumer finds a gateway offering on the broker and calls the gateway with the ticket it got. The gateway, as the provider, checks who is calling.

**How it works.**
1. The consumer discovers, picks the gw offering and receives a ticket.
2. It calls the gateway with its workspace key and the ticket in a header (`x-gw-broker-ticket`), which gw never forwards upstream.
3. gw keeps the ticket with the call record, for reporting only: redacted from request listings, exports and logs. This needs a field of its own: a ticket is longer than the 64 characters an `x-gw-tags` value keeps, and tags are stored as plain analytics data.
4. The bridge verifies tickets (`POST /v1/tickets/verify`). It reports about clients from what gw saw: `good_standing`, `abuse` (limits refused repeatedly), `non_payment` (from billing, where there is one).
5. Optionally, an offering sets `min_client_reputation`, so clients with a poor record do not find it.

**Attachment.** gw: request headers (`GatewayParams::from_headers`), a ticket field in `UsageEvent` (phase 2). Broker: tickets and client reports.

**Risks.** A ticket is a credential of sorts: neither side logs it.

### 3.3 R2: the broker as a source for the model repository

**What.** The broker becomes one more source for what the platform team curates in gw's model repository.

**Two depths:**
- **R2a, prices.** The broker serves its prices in gw's existing `pacod` feed format (`{"currency", "models": {"<model id>": {"input", "output", …, "variants"}}}`). A gw `PriceJob` source of kind `feed` points at it. The price tracker then works unchanged: within its bound it applies a price, beyond it proposes. Needs one read endpoint in the broker; nothing in gw.
  - The feed carries a period's base rates, and each time window's rates as a variant named after the window (`off-peak`). It never carries "the price in force now", which would flip at every window boundary and churn the tracker.
  - The feed format has no hours, so when a window applies is set once in gw's `windows` by hand, or by R2b later.
- **R2b, models.** Broker resources of kind `model` or `embedding`, with an interface gw speaks, become discovery proposals (`new`, `update`, `gone`). An admin accepts a proposal, which creates a draft model as today; credentials are added by hand, because the broker never holds any. Needs a proposal source in gw's core.

**Attachment.** gw: price tracker (R2a), `discovery_proposals` (R2b). Broker: discovery with filters (kind, protocols, regions, residency, `no_training_use`), the watch stream.

**Risks.** Two sources for one price; the price tracker's source order decides. Model names differ between catalogues; a link field (`broker_resource_id` on `Model`) makes the mapping explicit.

### 3.4 R3: the gateway as a witness

**What.** The gateway sees every call: outcome, timing, tokens, cost. That makes it the most trustworthy source of `reliability`, `performance` and `accuracy` evidence about the upstreams it calls.

**Two variants:**
- **R3a, the gateway as the client.** The bridge holds a ticket of its own for each upstream that is a broker resource (from R2's link), as a broker consumer. It reads gw's analytics (`/analytics/ranking`, `/requests/export`) and posts usage reports for a sample of calls, within 120 per resource per hour. Works with the broker as it is. The evidence counts as one well-informed client's.
- **R3b, the delegated witness.** A new broker permission `reports.witness` lets a principal confirm reports on another client's ticket for resources it carried. The consumer's own ticket then carries the gateway's observation, so the evidence is first-hand and per consumer. Needs a broker change and the ticket in `UsageEvent` (R1b).

**The same-organisation rule.** An organisation's gateway reporting on that organisation's own offerings does not count. This is intended: no one raises their own reputation.

**Mapping gaps.** `upstream_first_byte_ms` is the provider's response head, not the first token. `ms_per_token` must be derived (`upstream_stream_ms` over output tokens). Section 5 has the mapping; a first-token time in `UsageEvent` would close the gap.

### 3.5 R4: pools defined by discovery

**What.** A template's pool is a stored discovery query rather than a fixed list (gw catalogue RTE-01, phase 2: "stored discovery query").

**How it works.**
1. A pool entry holds a query: kind, capabilities, regions, residency, `max_price`, `min_reputation`, and a sort.
2. gw keeps a local mirror of the broker's answers, refreshed by the watch stream. It resolves the query when it builds its snapshot, never during a call.
3. Only resources linked to repository models with credentials can enter a pool.
4. **Mode `propose`** (the default): a change in the answer becomes a proposal on the template; an admin accepts it as a new template version. **Mode `follow`**: the pool follows the answer, each change recorded as a config change with audit.
5. Templates that name their models are untouched.

**Attachment.** gw: `TemplateSpec`, `routing/select.rs` (`bind`). Broker: discovery, watch.

**Risks.** A silent switch of model changes outputs, which is exactly what assessment §4.10 warns about. Hence `propose` by default, and `follow` only for templates that declare themselves interchangeable (embeddings, or self-hosted replicas of one model).

### 3.6 R5: leased upstreams

**What.** Self-hosted capacity (vLLM and other servers, GPU nodes) announces itself to the broker as it starts and stops. gw routes to whatever is there now (gw catalogue RTE-11 and BRK-02: registration with a heartbeat TTL).

**How it works.**
1. Each node announces with the broker's client: kind `model`, interface `openai-chat`, its inventory (GPU, engine), its capacity.
2. gw admits leased resources **from its own organisation only** into a pool of a template marked for it, as instances of one repository model.
3. A lease that ends removes the node from the pool at the next snapshot; heartbeats with `draining` order it last; the breaker still applies.
4. Private addresses need gw's egress allow-list (`PGW_EGRESS_ALLOW_PRIVATE`).

**Attachment.** gw: provider and model rows of a leased family, the breaker. Broker: leases, live state, the watch stream.

**Risks.** This is the one place where gw would change its upstreams without an admin: confined to its own organisation's hardware, behind an explicit setting.

### 3.7 R6: agents and MCP

**Now.** pacod-ap's agents can use the broker's MCP server (`/mcp`, tools `discover_resources` and `describe_resource`) directly to find tools and models at run time. They then call models through gw as today, with a `@template` or a model id.

**Later.** gw's planned virtual MCP server (catalogue MCP-01, ING-13 asks for exactly these two tools) fronts the broker's tools instead of building its own discovery. One engine answers both, as BRK-09 asks: "the router uses the same engine".

### 3.8 R7: the consoles

- **gw console.**
  - A server-only broker client behind an optional `BROKER_URL` (the `TESTER_URL` pattern in `ui/src/env.ts`).
  - Model pages show the broker's reputation and the prices other providers ask for the same model.
  - Organisation pages get an "offered on the broker" switch (R1).
  - The review queue shows proposals from the broker (R2b, R4).
  - Without `BROKER_URL`, the pages say the broker is not set up.
- **broker console.** Shows gw offerings like any provider, with the gateway's organisation.

### 3.9 Options rejected

| Option | Why not |
|---|---|
| A gw plugin as the integration | Plugins are compiled in and run on every call; the broker must stay off the request path. |
| The broker choosing an upstream per call | Conflicts with pinning, and puts the broker on the request path. |
| The broker holding upstream credentials | The broker holds no secrets; gw's sealed credentials stay in gw. |
| Reputation as an automatic re-ranking of pools | Assessment §4.10: silent switches change outputs. Reputation filters (R4) or informs (R7). |

## 4. Mechanisms

| | M1: a bridge service (recommended first) | M2: inside gw's core |
|---|---|---|
| **What** | `gw-broker`, a small service beside `gw-analytics`. It reads gw's management API and analytics, and talks to the broker with `pacod-broker-client` | Modules in `pacod-gw-core`: a broker mirror, proposal source, pool resolution |
| **Roles it can carry** | R1, R2a, R3a, R7 data; R1b once gw records tickets | R1b's and R3b's ticket field, R2b, R4, R5 |
| **gw change** | None in the core; one role in `authz/gw.yaml` (`broker_bridge`: read models, prices, templates, health and analytics) | Core code, migrations, snapshot build |
| **Failure** | Bridge down: offerings disappear with their leases; gw serves on | gw depends on a mirror it must keep when the broker is down |
| **Identity** | A pacod-id service account: `broker.provider@organisation` for each offering organisation, `broker.consumer` for R3a, `broker_bridge` in gw | gw's own service account |

**Where the bridge lives.** In pacod-gw, as `crates/pacod-gw-broker` with service `gw-broker`. It maps gw's types, which change with gw. `pacod-broker-client` is vendored at a pinned commit, as pacod-id's crates are (`vendor/pacod-id`).

**Leases and instances.** One bridge holds the leases. Run several for availability and they share one lease per organisation through gw's job lease (`store.job_lease`), so one announces at a time.

## 5. Mappings

### 5.1 Kinds and interfaces

| gw `ModelKind` | Broker `kind` |
|---|---|
| Chat | `model` |
| Embedding | `embedding` |
| Rerank, Speech, Transcription, Image, Realtime | `other:rerank`, `other:speech`, `other:transcription`, `other:image`, `other:realtime` |
| Decision | Not offered: for plugins only |

Interfaces: `pacod-gw` (the native API), plus `openai-chat`, `openai-responses`, `anthropic-messages` and `gemini` for the dialects the gateway translates.

### 5.2 Prices

| gw `Price` | Broker `pricing` |
|---|---|
| `currency` | `currency` (no conversion) |
| `rates` (token rates per million tokens, others per unit) | `rates` (the same units; the broker's rates mirror gw's) |
| `effective_from` | a period's `valid_from`; a new gw price version starts a new period |
| `windows` (a variant on weekdays, `hours_utc`) | a window named after the variant, with its rates, on those weekdays and hours; the pricing's `timezone` is `UTC` |
| `variants` not tied to hours (`batch`, `flex`) | No equivalent: a caller chooses them per request. The default rates are announced, and the variants named in the period's `note`. An organisation that sells batch separately offers it as its own resource. |
| `tiers` (`above_input_tokens`) | `tiers` |

The broker refuses a price that would change the past (`check_not_retroactive`). The bridge therefore announces a gw price change from its `effective_from` on, never backdated.

Broker to gw (R2a): the `pacod` feed (`GET /v1/feeds/prices`) carries a period's base rates, each window's rates as a variant named after it, and the windows themselves as UTC weekdays and hours (`FeedRates.windows`), worked out for the current week. A window in another zone therefore follows a change of daylight saving time at the next read of the feed. Tiers pass by input tokens only. Overrides (one-off spot prices), commitments and compute rates do not pass through the feed.

### 5.3 Life cycle and live state

| gw | Broker |
|---|---|
| Active | announced, `active` |
| Deprecated | announced, its sunset in the description; `degraded` from `sunset_at` |
| Retired, Disabled | not in the next announcement |
| Every credential's breaker open | heartbeat `degraded` |
| Calls in flight (`TargetState`) | heartbeat `in_use` |
| First-byte average (`TargetState`) | heartbeat `ttft_ms` (first byte, not first token) |

### 5.4 Usage reports

| `UsageEvent` | Usage report |
|---|---|
| `status` ok | `ok` |
| `http_status` 429 | `rate_limited` |
| `http_status` 5xx, `error` | `server_error` |
| `http_status` 4xx | `client_error` |
| `timeout` | `timeout` |
| `rejected` | `refused` |
| `cancelled` | not reported: the caller left |
| `upstream_first_byte_ms` | `ttft_ms` (an approximation until gw records the first token) |
| `upstream_stream_ms` ÷ output tokens | `ms_per_token` |
| `cost`, `currency` | `price_charged`, only when the currency is the resource's |

## 6. Scenarios

### 6.1 An organisation offers its fine-tuned model
Roles: R1, R7; R1b from phase 2.
1. Acme runs a fine-tuned model behind template `@acme-legal` on the gateway. It switches "offered on the broker" on for that template, visibility `platform`.
2. The bridge announces it with Acme's EUR prices and live state.
3. Globex's application discovers it (`kind: model`, residency `eu`), reads the price, asks Acme for a workspace key at `obtain_url`, and calls the gateway. From phase 2 it quotes its ticket, and Acme's bridge reports how Globex behaved as a client.
4. Globex rates the model and reports its calls with its ticket; that evidence counts. Evidence from Acme's own gateway about Acme's own offering does not.
5. Acme's console shows the offering's reputation; Globex's shows Acme as a provider.

### 6.2 The platform team keeps the catalogue current
Roles: R2a, then R2b.
1. The broker lists 40 models from several providers. A gw price source of kind `feed` points at the broker's feed for each provider.
2. A price that moves within the tracker's bound is applied; a larger one becomes a proposal.
3. With R2b, a new model on the broker appears in gw's review queue as a `new` proposal. The team accepts it, adds a credential, probes it and activates it as today.

### 6.3 Batch work follows off-peak prices
Roles: R2a, then R4 (propose).
1. A provider announces off-peak rates from 22:00 to 06:00 UTC. Through the feed, gw's `Price` gets the off-peak rates as variant `off-peak`; the platform team adds the `windows` entry with those hours once.
2. A batch template uses the `cheapest` algorithm over three models; at night the off-peak model leads.
3. With R4, the template's pool is a query (`max_price.input ≤ 0.5`, residency `eu`). When a cheaper model appears, the template gets a proposal; nobody's interactive template moves.

### 6.4 A self-hosted fleet grows and shrinks
Role: R5.
1. Acme starts four vLLM nodes in the morning; each announces itself with the Rust client and its GPU inventory.
2. gw adds them to template `@acme-local` as replicas of one repository model; `least_in_flight` spreads the load.
3. In the evening two nodes stop with a goodbye; one crashes, and its lease runs out after its TTL. The pool shrinks at the next snapshot; the breaker covered the gap.

### 6.5 A flaky upstream loses reputation
Roles: R3a, R4.
1. An upstream starts failing one call in ten. gw's breaker orders it last within each call's failover; the bridge's sampled reports lower its `reliability`.
2. Its broker score falls below a template's `min_reputation`; with R4 in `propose`, the template gets a proposal to drop it.
3. Templates that pin it keep it. Their owners see the score in the console and decide.

### 6.6 An agent finds a tool at run time
Role: R6.
1. A pacod-ap agent needs a weather tool. It calls the broker's MCP tool `discover_resources` with `kind: mcp_server`, `tags: [weather]`.
2. It connects to the server found, quoting its ticket, and later rates it.
3. Its model calls still go through gw with its template.

### 6.7 When something is down

| Down | Effect |
|---|---|
| Broker | Calls through gw are unaffected. The bridge retries; leases run out and gw offerings disappear from the broker until it is back (intended: the broker must not show what it cannot vouch for). R4 pools keep their last resolution. |
| Bridge | gw unaffected. Offerings disappear with their leases; evidence pauses. |
| Gateway | Its offerings fail and disappear with their leases; consumers' discovery shows the rest. |
| pacod-id | The broker serves public discovery; gw serves from its snapshot. Both as today. |

## 7. Trust and security

| Topic | Rule |
|---|---|
| **Identities** | The bridge has its own pacod-id service account: `broker.provider@organisation` for each organisation that offers, `broker.consumer` for R3a, `broker_bridge` in gw. No shared secrets between gw and the broker. |
| **Self-promotion** | The broker's same-organisation rule ignores an organisation's evidence about its own offerings, from the gateway too. |
| **Tickets** | Carried in a header that neither side logs or forwards upstream. |
| **Visibility** | Offerings default to `organisation:<id>`. `platform` is a deliberate choice per offering. |
| **Credentials** | Stay sealed in gw. The broker publishes only where access is obtained. |
| **Egress** | R5's private endpoints need gw's egress allow-list, per address. |

## 8. Phases

| Phase | What | Changes |
|---|---|---|
| **P0: validate demand** | Ask which organisations would offer, which would consume, and which templates are interchangeable. Assessment §4.10, point 1. | None |
| **P1: no change in gw's core** | The bridge with R1 (offerings), R2a (prices by feed), R3a (sampled evidence); R6 directly; R7 as links. | broker: the price feed endpoint. gw: `crates/pacod-gw-broker`, service `gw-broker`, role `broker_bridge`, console `BROKER_URL`. |
| **P2: small changes in gw's core** | R1b (the ticket field in `UsageEvent`, verification and client reports), R2b (the broker as a proposal source), `broker_resource_id` on `Model`, a first-token time in `UsageEvent`, R3b (the delegated witness) | gw core; broker: `reports.witness` |
| **P3: if P1 and P2 show value** | R4 (pools by discovery), R5 (leased upstreams), R6 through gw's virtual MCP server | gw core and console |

**What P1 proves.** Whether organisations offer and use each other's models, and whether gateway evidence moves reputation usefully. Both are measurable before gw's core changes.

## 9. Changes per repository

| Repository | P1 | P2 and later |
|---|---|---|
| **pacod-broker** | A price feed in gw's `pacod` format (a read endpoint: `make openapi`, the three clients, a scenario). A scenario with a gateway as provider and witness. | `reports.witness` and its scenario |
| **pacod-gw** | `crates/pacod-gw-broker` (vendored `pacod-broker-client`), the `gw-broker` compose service, the `broker_bridge` role (`make authz`), console `BROKER_URL` | The proposal source, `broker_resource_id`, `UsageEvent` fields; then pools by discovery and leased upstreams |
| **pacod-demo** | Service `gw-broker` on `pacod-net` and `pacod-admin-net`; service accounts for the bridge and the core, and one per organisation, made by the bootstrap; secrets in `init-secrets.sh` | — |
| **pacod-id** | None | — |

## 10. Open questions

1. **Offer templates or models?** Templates are stable and hide upstream changes; models are what consumers compare. The concept recommends templates, with models where an organisation runs its own.
2. **Offer across organisations at all?** If only an organisation's own teams consume, R1 reduces to visibility `organisation:<id>`, and R3's evidence cannot count (same organisation).
3. **Opt in per organisation or per offering?** The concept recommends per organisation, with a list of offerings.
4. **Which prices win** when a provider's own feed and the broker's disagree? The price tracker's source order decides; the concept recommends the provider's own source first.
5. **May R5 ever admit hardware outside the gateway's organisation?** The concept says no.

## 11. What was built

All three phases, on the branches `broker-integration` of pacod-gw and pacod-demo. Phase 0 is the bridge's metrics (`gwb_*`, scraped as job `pacod-gw-broker`): offerings, announcements, reports sent and counted, tickets seen.

### 11.1 Where the build differs from the concept

| Concept | Built | Why |
|---|---|---|
| Offerings marked by template tags (`broker.offer=…`) | The bridge's file (`GWB_CONFIG_FILE`: organisations, their broker tokens, offerings) and, from phase 3, a template's own `broker_offer { visibility, summary }`, the console's switch "Offer on pacod-broker" | Tags are declared per workspace and added to every call: the wrong place for a catalogue setting. |
| One pacod-id service account for the bridge, `broker.provider` in each organisation | One token per offering organisation, holding `broker.provider` there; the bridge's own token holds `broker.consumer` and `broker.witness` | Each organisation announces as itself, so the same-organisation rule keeps its own members' calls out of its reputation. Provider ids keep today's derivation. |
| A new gw role for phase 1 | Phase 1 reads with a platform `viewer`; phase 2 adds `broker_bridge` (`org.read`, `workspace.read`, `broker.read`), which alone reads the calls that quoted a ticket (`/broker/calls`) | A viewer already reads models, prices, templates, routing and requests. |
| Templates or models offered | Templates only, the active ones; the first pool model gives `kind` and prices | The concept's recommendation (section 10, question 1). |
| Interfaces `pacod-gw`, `openai-chat`, `openai-responses`, `anthropic-messages`, `gemini` | `pacod-gw`, `openai-chat`, `anthropic-messages`, `gemini`; `model.upstream_id` is `@<template>` | The names a caller sends to reach a template. |
| Deprecated models announced with their sunset | Not built: an offering follows its template's state | Templates have no sunset. |
| Several bridges sharing leases through gw's job lease | Not built: run one bridge | One bridge per installation is enough for the demand phase 0 measures. |
| R5 as an organisation setting `broker_leases { template, query }` | `pool_from.own_leases` on the template | One place for a template's members, whether found or leased. |
| A first-byte time as `ttft_ms` | `Timing.first_token_ms`, the first chunk with content per dialect; the bridge reports it, and the first byte only where it is missing | Phase 2 records the first token, as planned. |
| Console `BROKER_URL` as a link | A Broker page under Platform, from the core's `/broker/status` and `/broker/resources/{id}` | Users' tokens carry gw's audience; the core asks the broker with its own token. |
| Helm | Not done | No Helm in the environment the work was done in; compose only. |

### 11.2 Where each role lives

| Role | pacod-gw | Settings |
|---|---|---|
| R1 offerings, live state | `crates/pacod-gw-broker` (`offer.rs`, `bridge.rs`): announcements rebuilt on `config` events and every `GWB_REFRESH_SECONDS`, updated under the same lease (`Running::update`) | `GWB_*` (`config.rs`) |
| R1b front door | `x-gw-broker-ticket` on a call becomes `UsageEvent.broker_ticket` (never forwarded upstream, never in the audit log); the bridge reads `/broker/calls`, checks each ticket as the offering's provider, reports `good_standing` once a day per client and `abuse` past 20 refused calls a day (`front_door.rs`) | role `broker_bridge` |
| R2a prices | the broker's `GET /v1/feeds/prices` as a `pacod` feed in gw's price tracker | a price job's URL |
| R2b proposals | the core's discovery run asks the broker for `model` and `embedding` resources at its providers' hosts; proposals carry `source = broker`; accepting one sets `Model.broker_resource_id` | `PGW_BROKER_URL`, `PGW_BROKER_TOKEN`, `PGW_BROKER_POOL_SECONDS` |
| R3a evidence | the bridge reports sampled calls to linked upstreams under its own ticket (at most `GWB_SAMPLE_PER_HOUR` per resource and hour) | links in the bridge's file, or `broker_resource_id` |
| R3b witness | the bridge reports offering calls on the consumers' own tickets as `reports.witness` | the bridge's token holds `broker.witness` |
| R4 pools | `TemplateSpec.pool_from { query, mode: propose or follow, max, own_leases }`; a `broker_pool` object per template holds the resolved and the proposed members; members come after the template's own, capped at 16 | the template editor's "pacod-broker" fieldset |
| R5 leased upstreams | with `own_leases`, the organisation's own leased `openai-chat` resources become providers `leased-<12 hex>` and models `leased-…/<upstream>`, granted to it; a lease that ends disables only that template's members | the egress policy still applies |
| R6 MCP | the data plane's `POST /mcp` forwards `initialize`, `tools/list` and `tools/call` for `discover_resources` and `describe_resource` to the broker, with a gw key | `PGW_BROKER_*` |
| R7 consoles | the Broker page, the discovery queue's "Found by" column, the template editor | — |

### 11.3 Not verified where it was built

- gw-analytics against ClickHouse: migration `0014_broker.sql` (the ticket column and `first_token_ms`) and the reports' `broker_ticket` filter are covered by unit tests only.
- The full demo stack: its files pass `docker compose config -q`, `sh -n` and parsing, but the stack did not run (no Docker daemon).
- Everything else ran: the bridge against a real broker, the core's integration tests against PostgreSQL (proposals, pools in both modes, a leased node serving and then disabled, MCP), and the console against a live core and broker.

