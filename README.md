<div align="center">

# pacod-broker

**Discovery of AI and compute resources: providers announce, consumers discover, both earn a reputation.**

Models, MCP servers, agents, tools and GPU capacity, each with its interfaces, prices over time,<br>
availability over time and the hardware it runs on: one small binary, no database, nothing to administer.

![Rust](https://img.shields.io/badge/Rust-000?style=flat-square&logo=rust)
![License](https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square)
![Self-hosted](https://img.shields.io/badge/deploy-self--hosted-2ea44f?style=flat-square)

</div>

---

pacod-broker is the capability broker of the original *AI Broker & Gateway* concept (pacod-gw's feature catalogue, section 12), built as a product of its own in the [pacod](https://github.com/pacod22) family. It sits beside [pacod-gw](https://github.com/pacod22/pacod-gw), signs people and services in through [pacod-id](https://github.com/pacod22/pacod-id), and reports to the same Prometheus and Grafana.

| | |
|---|---|
| **Providers announce** | `PUT /v1/announce` with a profile, an optional hardware and software inventory, and resources. Each announcement is a **lease**: a heartbeat keeps it, and without one the resources disappear on their own. Static entries come from a YAML catalogue. |
| **Consumers discover** | `POST /v1/discover`: filter by kind, words, capabilities, protocols, model limits, regions, data policy, GPU model, software version, **maximum price at an instant or over a span**, **availability over a span**, and **reputation**; sort within a kind by price, latency or reputation. Every result says why it matched. |
| **Time is first-class** | Prices have periods, recurring windows (off-peak, weekends), one-off overrides, volume tiers and commitments. Availability has validity, opening hours, maintenance and bookable slots. Ask *"what does it cost and is it free next Tuesday night?"* |
| **Reputation for both sides** | Providers and resources are rated on availability, reliability, performance within their kind, accuracy of their claims, lease discipline, stability, quality ratings and trust. Clients are rated on identity, conduct, settlement, commitment keeping, the honesty of their feedback and tenure. Reports count only with a **broker-signed interaction ticket**. See [docs/reputation.md](docs/reputation.md). |
| **Agents use it directly** | `/mcp` serves `discover_resources`, `describe_resource` and `describe_provider` as MCP tools. |
| **Small and fast** | 12.7 MB release binary, about 15 MB image (distroless, static), starts in under 100 ms, 23 MB idle with the example catalogue. Filtered discovery takes about 0.2 to 1.2 ms at 10 000 resources; queries matching the whole catalogue take up to about 3.5 ms. |
| **No administration needed** | Environment variables only, all optional. No database: leases are soft state, and what must survive a restart (static entries, decisions, reputation evidence) is a few files in `BROKER_STATE_DIR`. |
| **Clients in three languages** | Rust, Python and TypeScript libraries, each with a command line: a provider that announces and stays announced (heartbeats, re-announcing, a clean goodbye), and a consumer that discovers, watches and reports. See [docs/clients.md](docs/clients.md). |
| **A console that docks to it** | An optional SvelteKit console ([ui/](ui/)) on pacod-id's design and sign-in, docking to one or several brokers' admin APIs. A broker never needs it. |

## Try it

```bash
make run-catalog
```

The broker starts in open mode with [the example catalogue](examples/catalog.example.yaml): public API on `127.0.0.1:18060`, admin API on `127.0.0.1:18061`, metrics on `127.0.0.1:19160`. In another terminal:

```bash
cargo run -p pacod-broker-client --bin broker-cli -- announce examples/announcement.example.yaml
```

```bash
cargo run -p pacod-broker-client --bin broker-cli -- --client me discover --kind model --currency EUR --sort price:input
```

```bash
examples/mcp.sh
```

```bash
make console
```

The console comes up on <http://127.0.0.1:5173>. `make smoke` runs the end-to-end check.

With Docker, `make dev-up` builds and starts the broker and the console from the working tree, and `make up` adds the edge and monitoring of [docker-compose.standalone.yml](deploy/docker-compose.standalone.yml). In the full pacod stack the broker joins [pacod-demo](https://github.com/pacod22/pacod-demo) like the other products ([docs/integration.md](docs/integration.md)).

## Documentation

| | |
|---|---|
| [docs/concept.md](docs/concept.md) | The design: architecture, data model, discovery, access, what is deferred and why |
| [docs/reputation.md](docs/reputation.md) | The reputation model: criteria, computation, evidence, uses |
| [docs/clients.md](docs/clients.md) | The Rust, Python and TypeScript clients and their command lines |
| [docs/api.md](docs/api.md) | The public API, the admin API and MCP, with examples; [docs/openapi.json](docs/openapi.json) is generated from the code |
| [docs/runbook.md](docs/runbook.md) | Configuration, deployment, monitoring, alerts, backups |
| [docs/integration.md](docs/integration.md) | Joining the full stack, and the follow-ups in pacod-gw, pacod-id and pacod-demo |

## Layout

| Path | What |
|---|---|
| `crates/pacod-broker-model` | The data model: providers, inventories, resources, prices and availability over time. Pure. |
| `crates/pacod-broker-index` | The immutable index and the query engine. Pure; criterion benchmark. |
| `crates/pacod-broker-reputation` | Evidence, decayed aggregates, scores; the evidence log. |
| `crates/pacod-broker` | The binary: registry and leases, APIs, MCP, tickets, pacod-id, metrics, state. |
| `crates/pacod-broker-logging` | The pacod log format, copied from pacod-gw. |
| `crates/pacod-broker-client`, `clients/python`, `clients/typescript` | The clients and their command lines. |
| `ui/` | The console (SvelteKit 3). |
| `authz/broker.yaml` | Roles and permissions for pacod-id. |
| `deploy/`, `platform/` | Images, compose files, the alert rules and dashboard; fragments for the shared Caddy and Prometheus. |
| `vendor/pacod-id` | pacod-id's client crates at a pinned commit (`scripts/vendor-pacod-id.sh`). |

## License

Apache License 2.0.
