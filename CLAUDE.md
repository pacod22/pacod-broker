# pacod-broker: notes for working on the code

A discovery broker for AI and compute resources, with prices and availability over time and reputation for providers and clients. Read [README.md](README.md) first, then [docs/concept.md](docs/concept.md). The reputation model is [docs/reputation.md](docs/reputation.md).

## Layout

| Path | What |
|---|---|
| `crates/pacod-broker-model` | Types and validation; time resolution of prices (`pricing.rs`) and availability (`availability.rs`) on `schedule.rs`. Pure. |
| `crates/pacod-broker-index` | `Index::build` and `Index::discover`. Pure; `benches/discover.rs`. |
| `crates/pacod-broker-reputation` | `Engine::record`, `Engine::compute`; the evidence log and snapshots. |
| `crates/pacod-broker` | The binary: `config.rs`, `registry.rs`, `api/` (public, admin, MCP, views), `auth.rs`, `identity.rs` (pacod-id), `tickets.rs`, `state.rs`, `openapi.rs`, `server.rs`. |
| `crates/pacod-broker-logging` | Copied from pacod-gw-logging; keep the format of pacod-demo/docs/LOGGING.md. |
| `crates/pacod-broker-client`, `clients/python`, `clients/typescript` | The clients (docs/clients.md): same design in three languages; keep them in step. The TypeScript types come from docs/openapi.json. |
| `ui/` | The console, SvelteKit 3, after pacod-gw's console. |
| `authz/broker.yaml` | Roles and permissions; the binary embeds it, the console generates `authz.generated.ts` from it. |
| `vendor/pacod-id` | pacod-id's three client crates at a pinned commit. Never edit; run `make vendor-pacod-id`. |

## Commands

```sh
make verify        # fmt, clippy -D warnings, tests, OpenAPI and authz drift, console check/test/build
make run-catalog   # the broker with the example catalogue on 127.0.0.1:18060/18061
make console       # the console docked to it
make smoke         # end to end against a running broker
make bench         # discovery at 10 000 resources
```

## Rules

- **The hot path takes no lock.** Queries read `ArcSwap<Index>` and the live atomics; heartbeats touch atomics only. Anything that changes records goes through the registry and a rebuild.
- **Discovery never waits for pacod-id.** Keep it that way.
- **Ids are derived, not random** (`ids.rs`), so providers and resources keep their reputation across restarts. Never change the derivation without a migration of the reputation files.
- **Prices are never retroactive** (`Pricing::check_not_retroactive`). Rank only within one kind. Convert no currency.
- **Reputation counts only evidence with a ticket**, or what the broker observes itself.
- **Configuration.** A new variable goes into `config.rs` (`PLAIN`) and `deploy/.env.example`; a test checks that the two agree.
- **API changes** need `make openapi` (docs/openapi.json, the console's and the TypeScript client's types), and the three clients changed alike; `make clients-test` runs them against a real broker. **Role changes** in `authz/broker.yaml` need `make authz`.
- **Networks and names** follow pacod-demo/docs/NETWORK.md: `pacod-broker` (shared name), `broker-*` for its other services; the admin listener binds the alias `pacod-broker-admin` on `pacod-admin-net`.
- **Never log** tokens, tickets, request bodies or descriptions.
- **Code style.** `rustfmt.toml` (120 columns). Comments say why, in full sentences. `cargo fmt` only on this workspace's crates (`make fmt`), never on `vendor/`.
- **Documentation** is British English, short declarative sentences, tables, numbered sections. Keep docs/concept.md, docs/api.md and docs/runbook.md true to the code.
- **Commits** are titled `<Area>[, stage N]: <what changed>` and say why in the body.
