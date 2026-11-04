# pacod-broker: the scenario and load suite

The unit and API tests prove the parts. This suite runs the real broker binary against many configurations of providers and consumers:

- **Scenarios** (`tests/scenarios/*.yaml`) set up a broker, providers and consumers, then run steps whose answers must meet expectations.
- **Load runs** (`tests/load/*.yaml`) put many providers and consumers on one broker and measure it against targets.

Both drive the broker through the Rust client ([clients.md](clients.md)). Scenarios in `oidc` mode bring a mock of pacod-id's internal API.

## 1. Running it

| Command | What |
|---|---|
| `make suite` | Every scenario against the release build. The report goes to `target/suite`. |
| `make suite-load` | The load runs; `SCALE=0.2` runs them at a fifth of their size and length |
| `make suite-one S=lifecycle` | The scenarios and load runs whose name contains `S` |
| `cargo test -p pacod-broker --test scenarios` | Every scenario under cargo, against the binary that build made. `make verify` runs it with the other tests. `SCENARIO=<name>` runs one. Its report goes to `target/tmp/scenarios`. |
| `cargo run -p pacod-broker-suite -- --help` | The runner's options: `--broker`, `--only`, `--jobs`, `--scale`, `--out` |

**How a run works:**
- **One broker per scenario.** Each scenario gets its own broker process, on free local ports, with only the scenario's environment. Four scenarios run at once by default (`--jobs`).
- **Which binary.** The runner takes `--broker`, else `BROKER_BIN`, else the newer of `target/release` and `target/debug`. It takes the newer one so that a stale release build is never tested by mistake.
- **Fast defaults.** The suite sets `BROKER_REBUILD_MS=5`, `BROKER_REPUTATION_EVERY=1` and `BROKER_LEASE_TTL_MIN=2`, so that leases run out and scores move within seconds. A scenario's `env` overrides them.
- **Restarts are real.** `restart_broker` stops the process (SIGTERM, or SIGKILL with `graceful: false`) and starts it again on the same ports and state directory.

**The report.** `target/suite/report.md` and `report.json` hold:
- each scenario's steps, with time and outcome;
- for a failure, the step, what was expected, what came back, and the broker's log tail;
- each load run's percentiles, the broker's memory and CPU, and its targets.

The runner exits with 1 when anything fails.

## 2. Scenarios

| # | Scenario | Configuration | Proves |
|---|---|---|---|
| 1 | `lifecycle` | Open mode, one provider, one consumer | Announce, discover, live state through heartbeats, a goodbye removes, ids stay stable |
| 2 | `ranking-within-a-kind` | Three providers of one model at different prices and speeds, plus one in USD | Sorting by price, TTFT and reputation; `kind` is required to sort; the currency filter |
| 3 | `prices-and-hours` | Off-peak windows, an override, a future period, opening hours, maintenance | `at` and `during`: prices in force, coverage `full` and `any`, `min_capacity` |
| 4 | `inventory-matching` | GPU and software inventories | Hardware and software filters, version ranges |
| 5 | `lost-leases` | A lease the broker forgets; a provider that crashes | The provider announces again with the same ids; a crash ends the lease after its TTL; watch events |
| 6 | `lease-discipline` | Graceful and crashing providers over several rounds | Reputation `lease_discipline` separates them |
| 7 | `honest-and-dishonest` | One provider honest about latency and price, one not | `accuracy` and `performance` separate them |
| 8 | `bad-clients` | Providers report a client; a resource with `min_client_reputation` | The client loses access; a reset gives it back |
| 9 | `fake-feedback` | Ratings without a ticket, with another client's ticket, twice in the hour, floods | None of them count |
| 10 | `approval-and-blocking` | `BROKER_APPROVAL=new_providers` | Pending, approve, block (leases end, 403), unblock |
| 11 | `limits` | Small `BROKER_MAX_*` values | Each limit refuses with its problem; the broker stays healthy |
| 12 | `restart-with-state` | A state directory, a static entry, decisions, reputation evidence | All survive a restart; leased providers announce again by themselves |
| 13 | `catalogue-reload` | A catalogue file edited mid-run, then SIGHUP | New entries appear; a broken file keeps the old entries |
| 14 | `organisations` (oidc) | Two organisations; resources `public`, `platform` and `organisation:` | Each consumer sees exactly what it should; anonymous callers see public entries; a hidden inventory never matches |
| 15 | `roles` (oidc) | A provider, a consumer, a person, a viewer, an admin, a service account with no role | Who may announce, discover, report and see the admin API (401, 403, 200); a provider reports only on its own resources' tickets |
| 16 | `same-organisation` (oidc) | A consumer in the provider's own organisation | Its ratings and reports do not count |
| 17 | `identity-down` (oidc) | The mock pacod-id stops mid-run | Discovery and heartbeats go on; a token the broker has not seen gets 503; both recover |
| 18 | `mixed-fleet` | 30 providers of every kind, consumers with different queries | Every result equals an independent filter of the announced data (the oracle) |

**A new feature gets a scenario,** or a step in one that fits.

### 2.1 The format

```yaml
name: ranking-within-a-kind
about: One sentence: what is set up and what it proves.
broker:
  auth: open                       # open (default) | oidc
  env: { BROKER_PUBLIC_DISCOVERY: "true" }   # any BROKER_* setting
  state_dir: false                 # a state directory, kept across restart_broker
  catalog: catalogs/one.yaml       # a catalogue file (static entries)
  admin_token: secret              # open mode: require this admin token
identity:                          # oidc only: the mock's organisations and principals
  organisations: [acme, globex]
  principals:
    - { name: farm-svc, kind: service, organisation: acme, roles: [broker.provider@organisation] }
    - { name: alice, kind: user, roles: [] }        # every person holds broker.member
providers:
  - { name: cheap, from: announcements/llama.yaml, set: { "resources[0].pricing.periods[0].rates.input": 0.2 } }
  - { name: fleet, from: announcements/gpu-node.yaml, count: 10, vary: { "resources[0].pricing.periods[0].rates.per_gpu_hour": [1.0, 4.0] } }
  - { name: farm, from: announcements/llama.yaml, as: farm-svc }   # oidc: the principal it announces as
consumers:
  - { name: c1 }                   # open mode: the name it gives itself
  - { name: buyer, as: alice }     # oidc: the principal
  - { name: nobody, anonymous: true }
  - { name: late, as: farm-svc, fresh_token: true }   # oidc: a service token the broker has not seen
steps:
  - start: [cheap, fleet]
  - discover: { as: c1, query: { kind: model, sort: "price:input" }, expect: { order: [cheap/llama] } }
```

- **Templates.** `from` names an announcement in `tests/scenarios/announcements/`. `set` changes values by path. `count` makes `name-1` … `name-n`, and `vary` spreads a value evenly over them; whole bounds give whole numbers.
- **Names.** `provider/key` names a resource; the runner derives its id the way the broker does. A provider's name names it in steps.

### 2.2 Steps

| Group | Step | What |
|---|---|---|
| Providers | `start: [p, …]` | Announce and keep the lease alive |
| | `announce: { provider, status }` | Announce once; `status` expects a refusal |
| | `stop: [p]` / `crash: [p]` | End the lease with a goodbye; stop heartbeats without one |
| | `live: { provider, resource, status, in_use, ttft_ms }` | Live state for the next heartbeat |
| Consumers | `discover: { as, query, expect, status, within }` | A query and what its answer must be |
| | `resource: { as, id, expect, status, within }` / `provider_view: { as, provider, … }` | One resource; one provider |
| | `watch: { as }`, then `expect_events: { as, contains, within }` | Collect the change stream; the event types it must hold |
| | `report: { as, on, outcome, ttft_ms, ms_per_token, price_charged, times, ticket_of, no_ticket, accepted }` | Usage reports, with a ticket; `accepted` is how many must count |
| | `rate: { as, on, rating, ticket_of, accepted }` | A rating |
| | `report_client: { provider, about, on, kind, status }` | A provider reports about a client that holds a ticket for `on` |
| Broker | `admin: { action, target, side, file, status }` | `block`, `unblock`, `approve`, `verify`, `unverify`, `incident`, `evict`, `freeze`, `unfreeze`, `reset`, `reject_report`, `put_static`, `delete_static` |
| | `admin_access: { as, token, status }` | Who may see the admin API |
| | `restart_broker: { graceful }` | Stop and start the process |
| | `reload_catalog: { file }` | Replace the catalogue file and send SIGHUP |
| | `id_down` / `id_up` | The mock pacod-id answers 503, then answers again |
| Checks | `reputation: { expect: { "resource p/key": { score: ">0.6" } }, within }` | Scores through the admin API; also `provider p`, `client c` |
| | `metrics: { expect: { broker_leases_expired_total: ">=1" }, within }` | Sums of `/metrics` series |
| | `wait: 2s`, `wait_for: { as, gone, present, within }` | A pause; a resource appearing or disappearing |

**Expectations** (`expect`):

| Key | What |
|---|---|
| `total` | The number of results: a number or a comparison |
| `ids` | Exactly these resources, in any order |
| `order` | The first results, in this order |
| `contains` / `excludes` | These resources are (not) among the results |
| `fields` | Values at paths in the answer (`hits[0].price.source`, `components.quality.samples`) |
| `oracle` | The results are exactly what the runner's own filter of the announced data gives |

Values compare as equal (numbers within a small tolerance), or with `>=`, `<=`, `>`, `<`, `!=` before them, or `~text` for "contains".

**Eventual answers.** Changes reach the index with the next rebuild, a few milliseconds later. Steps with `within` therefore repeat until their expectation holds or the time runs out, and report the last failure.

## 3. Load runs

| Run | Shape | Targets |
|---|---|---|
| `steady` | 50 providers × 20 resources; 32 consumers mixing filtered queries; 60 s | Discovery p99 ≤ 2 ms inside the broker; at least 500 discoveries a second; no errors |
| `heartbeat-storm` | 2 000 providers, one resource each, a 5 s lease; 30 s | No lease runs out; heartbeat p99 ≤ 1 ms inside the broker; no errors |
| `big-catalogue` | 200 providers × 50 resources (10 000); 64 consumers without pause; 60 s | At least 400 discoveries a second; RSS ≤ 200 MB; rebuild p99 ≤ 1 s; no errors. Latency here is queueing. |
| `churn` | 300 providers that crash and come back; 16 consumers; 45 s | Rebuild p99 ≤ 500 ms; no answer holds a lease that had run out; no errors |

**Format:**

```yaml
name: steady
template: ../scenarios/announcements/llama.yaml   # its provider and first resource
providers: { count: 50, resources_each: 20, ttl_s: 30, kinds: [model, embedding], churn: { up: "3s..8s", down: "5s" } }
consumers:
  count: 32
  queries:
    - { weight: 4, query: { kind: model, sort: "price:input" } }
warmup: 3s
duration: 60s
targets: { server_discover_p99_ms: 2, discover_per_second: 500, errors: 0 }
```

**Targets:**

| Target | Measured |
|---|---|
| `discover_p99_ms`, `heartbeat_p99_ms`, `announce_p99_ms` | Over HTTP, at the client, at most |
| `server_discover_p99_ms`, `server_heartbeat_p99_ms` | Inside the broker, from its own histograms, at most |
| `discover_per_second` | Discovery throughput, at least |
| `errors` | Failed calls. Under churn, heartbeats to a lease that ran out are expected and not counted. |
| `rss_mb` | The broker's peak resident memory |
| `rebuild_p99_ms` | Index rebuilds, from the broker's histogram |
| `leases_expired` | Leases that ran out without a goodbye |
| `stale_hits` | Results holding a lease that had run out more than a second before |

**Open files.** Every provider and consumer of a run holds a connection, on the runner's side and on the broker's. The runner raises its open-file limit to the hard limit (macOS starts processes at 256), and the broker inherits it. When even the hard limit is too low, a run fails at once and says how far to raise it (`ulimit -n`), instead of measuring connections that never open.

**Measured or not.** The broker's memory and CPU come from `/proc`, or from `ps` where there is none (macOS); its own latencies come from `/metrics`. When one of them cannot be read, the run fails and says which, and the report shows "not measured", never 0.

**Over HTTP and inside.** The consumers and providers run on the machine the broker runs on and share its CPUs, and the consumers call again as soon as an answer arrives. Latency at the client is therefore mostly queueing: its mean is about the number of consumers divided by the throughput. The report shows it, but the latency targets are inside the broker, from its own histograms, which show what the broker itself takes. A full run belongs on a quiet machine; `SCALE=0.2` is a quick check.

## 4. The mock pacod-id

A real pacod-id needs PostgreSQL. The `oidc` scenarios therefore bring a mock (`crates/pacod-broker-suite/src/idmock.rs`) that answers exactly what the vendored pacod-id client calls:

| Endpoint | Answers |
|---|---|
| `POST /oidc/token` | The broker's own service token |
| `GET /oidc/jwks` | An ES256 key, made with `pacod-id-jose` |
| `PUT /internal/v1/catalog/broker` | Takes the manifest and checks it with `pacod-id-authz`, so a broken `authz/broker.yaml` fails the suite |
| `GET /internal/v1/authz/snapshot`, `/changes` | The scenario's role bindings |
| `POST /internal/v1/introspect` | The scenario's `pidt_…` service tokens |

People get signed access tokens (`at+jwt`, audience `pacod-broker`); service accounts get `pidt_` tokens. A principal `suite-admin` with `broker.admin` is always there, for the runner's own admin calls.

**Keeping the mock honest.** `crates/pacod-broker-suite/tests/idmock_contract.rs` runs the vendored `IdClient`, `AuthzCache` and `Authenticator` against the mock. A drift between the two shows up there, by name.

## 5. Writing a scenario

1. **Start from the nearest scenario.** Copy it, and keep one idea per scenario.
2. **Reuse a template** from `announcements/`, changing only what the scenario is about with `set`.
3. **State why** in `about`, and in comments above steps that are not obvious.
4. **Use `within`** wherever the answer follows a change: an announcement, a crash, a score.
5. **Check a refusal by its status** (`status: 422`), and check that the broker stays healthy afterwards.
6. **Run it alone** (`make suite-one S=<name>`), then with the rest.
