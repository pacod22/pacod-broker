# pacod-broker: runbook

## 1. Running it

| How | Command | Notes |
|---|---|---|
| The binary | `pacod-broker` | Open mode on `:8060`, nothing kept. `pacod-broker check` validates the settings and the catalogue file without starting. |
| One container | `docker run -p 8060:8060 -v broker-state:/var/lib/pacod-broker -e BROKER_STATE_DIR=/var/lib/pacod-broker pacod-broker:0.1.0` | The image is static and distroless, and runs as user 65532. |
| The broker's own stack | `make up` | The broker, the console, Caddy, Prometheus and Grafana ([deploy/docker-compose.standalone.yml](../deploy/docker-compose.standalone.yml)). |
| The full pacod stack | pacod-demo | See [integration.md](integration.md). |

**Images.** `make images` builds both. The Rust image builds with cargo-chef on Rust 1.99 and needs network access to crates.io and the Debian mirror.

## 2. Configuration

Configuration comes from environment variables only, and every variable is optional. [deploy/.env.example](../deploy/.env.example) lists them all with their defaults, and a test keeps that file and the code in step.

- **Unknown variables.** A `BROKER_*` variable the broker does not know stops it from starting.
- **All problems at once.** Every problem is reported in one go.
- **Env file.** `--env-file` (or `BROKER_ENV_FILE`) reads a file; the process environment wins over it.

| Group | Variables |
|---|---|
| Identity and listeners | `BROKER_ID`, `BROKER_LISTEN`, `BROKER_ADMIN_LISTEN`, `BROKER_METRICS_LISTEN` |
| Access | `BROKER_AUTH` (open, oidc), `BROKER_ADMIN_TOKEN[_FILE]`, `BROKER_PUBLIC_DISCOVERY`, `BROKER_APPROVAL` |
| pacod-id | `BROKER_ID_ISSUER`, `BROKER_ID_INTERNAL_URL`, `BROKER_ID_CLIENT_ID`, `BROKER_ID_CLIENT_SECRET[_FILE]`, `BROKER_ID_AUDIENCE` |
| What is kept | `BROKER_STATE_DIR`, `BROKER_CATALOG_FILE` |
| Leases and limits | `BROKER_LEASE_TTL_DEFAULT/MIN/MAX`, `BROKER_MAX_RESOURCES`, `BROKER_MAX_RESOURCES_PER_PROVIDER`, `BROKER_MAX_LEASES_PER_PROVIDER`, `BROKER_MAX_BODY_BYTES`, `BROKER_MAX_DESCRIPTION_BYTES`, `BROKER_REBUILD_MS` |
| Reputation | `BROKER_TICKET_KEY[_FILE]`, `BROKER_TICKET_TTL`, `BROKER_REPUTATION_EVERY`, `BROKER_REPUTATION_HALF_LIFE_DAYS`, `BROKER_REPUTATION_PRIOR_WEIGHT`, `BROKER_REPUTATION_WEIGHTS_PROVIDER`, `BROKER_REPUTATION_WEIGHTS_CLIENT`, `BROKER_MAINTENANCE_NOTICE` |
| Logging | `BROKER_LOG_FORMAT` (json, text), `RUST_LOG`, `SERVICE_NAME` |

The console's settings are listed in [ui/src/env.ts](../ui/src/env.ts) and [ui/.env.example](../ui/.env.example).

## 3. With pacod-id

1. **Register two clients in pacod-id's configuration** (pacod-demo's `config/pacod-id.toml` in the full stack):
   - a `service` client `pacod-broker-service` with `product = "broker"`, `audience = "pacod-broker"` and a secret of at least 32 characters;
   - a `confidential` client `pacod-broker-ui` with `redirect_uris = ["<console>/auth/callback"]` and the same audience.
2. **Set the broker's variables:**
   - `BROKER_AUTH=oidc`;
   - `BROKER_ID_ISSUER`;
   - `BROKER_ID_INTERNAL_URL=http://pacod-id-internal:8071`;
   - `BROKER_ID_CLIENT_SECRET`.

   The broker pushes `authz/broker.yaml` to pacod-id at start.
3. **Set the console's variables:**
   - `BROKER_UI_AUTH=oidc`;
   - `BROKER_ID_ISSUER`;
   - `BROKER_UI_CLIENT_SECRET`;
   - `BROKER_UI_SESSION_SECRET` (at least 32 characters);
   - `BROKER_CONSOLE_URL`.
4. **Give people roles** in pacod-id's administration: `broker.admin`, `broker.operator`, `broker.moderator` or `broker.viewer`. Every person may discover and rate already, through the implicit role `member`.
5. **Each provider becomes a pacod-id service account** with the role `broker.provider` (at the platform, or in its organisation), and announces with its `pidt_…` token.

If pacod-id is unreachable, discovery of public resources continues, and the admin API answers 503 `identity_unavailable`. That is the `BrokerIdentityUnavailable` alert.

## 4. What is kept, and backups

With `BROKER_STATE_DIR` set, the broker writes:

| File | What | When |
|---|---|---|
| `static.json` | Static entries added through the admin API | On each change |
| `decisions.json` | Providers approved, blocked, verified | On each change |
| `ticket.key` | The ticket signing key, unless `BROKER_TICKET_KEY` is set | Once |
| `reputation.log` | Reputation evidence, one JSON line each | Appended |
| `reputation.snap` | The reputation aggregates | Daily and at shutdown, after which the log starts again |

Files are replaced atomically. To back up, copy the directory; while the broker runs, the copy may lose the last few log lines, nothing more.

Leased entries are never written: after a restart, providers announce again on their next heartbeat (404 tells them to). Static entries come back from the catalogue file and `static.json`.

## 5. Sizing

| Resource | Figure |
|---|---|
| Memory | About 23 MB idle with a small catalogue. Allow roughly 2 to 4 KB per resource, plus its description and examples. The compose file limits the container to 256 MB. |
| Open files | One per connection: every provider holds one for its heartbeats, every consumer one while it calls. The broker raises its soft limit to the hard limit at start, and warns when that leaves fewer than 4 096. Outside a container, raise the hard limit (`LimitNOFILE` under systemd, `ulimit -n`). |
| CPU | Discovery is CPU-bound and lock-free, and scales with cores. Index rebuilds run off the request threads; a rebuild of 10 000 resources takes about 150 ms. With many announcements per second, raise `BROKER_REBUILD_MS` so more changes go into each rebuild. |
| Limits | `BROKER_MAX_RESOURCES` (100 000), `BROKER_MAX_RESOURCES_PER_PROVIDER` (5 000) and `BROKER_MAX_LEASES_PER_PROVIDER` (16) bound memory whatever providers send. |

## 6. Monitoring

| What | Where |
|---|---|
| Metrics | Port 9160, job `pacod-broker` ([platform/prometheus/broker.yml](../platform/prometheus/broker.yml)) |
| Dashboard | Grafana, folder `pacod-broker` ([deploy/grafana/dashboards/broker-overview.json](../deploy/grafana/dashboards/broker-overview.json)). The console's Metrics page shows the same figures from `PROMETHEUS_URL`. |
| Logs | stdout, one event per line, JSON by default (pacod-demo/docs/LOGGING.md). `make logs` |
| Health | `/healthz` (alive), `/readyz` (ready: fails while draining, and in oidc mode until pacod-id's feed is current) |

## 7. Alerts

[deploy/prometheus/rules/broker.rules.yml](../deploy/prometheus/rules/broker.rules.yml):

| Alert | Meaning | What to do |
|---|---|---|
| `BrokerDown` | Prometheus cannot reach the broker | Check the container (`make logs`). Providers' leases run out while it is down, and they announce again when it is back. |
| `BrokerServerErrors` | More than 1 % of requests fail with 5xx | Read the log for the failing route. |
| `BrokerIdentityUnavailable` | 503s for ten minutes: pacod-id's feed is not current | Check pacod-id and that the broker is on `pacod-admin-net`. Public discovery continues. |
| `BrokerLeaseChurn` | Many leases end without a goodbye | Providers crash, or their heartbeats do not arrive. Compare their heartbeat interval with `heartbeat_every_s`. |
| `BrokerDiscoverySlow` | Discovery p99 above 25 ms | A much larger catalogue than planned, or CPU starvation. Check the container's CPU limit. |
| `BrokerRebuildSlow` | Rebuilds take more than a second | Raise `BROKER_REBUILD_MS`, or the CPU limit. |

## 8. Operating tasks

| Task | How |
|---|---|
| Reload the catalogue file | `kill -HUP <pid>`, or `docker kill -s HUP pacod-broker` |
| Block a provider | Console, provider page, Block (or `POST /api/v1/providers/{id}/block`) |
| Approve new providers | Set `BROKER_APPROVAL=new_providers`; the console's overview lists them |
| Take back an unfounded report | Console, Moderation |
| Rotate the ticket key | Replace `BROKER_TICKET_KEY` or delete `ticket.key`, then restart. Tickets issued before stop working, so consumers discover again. |
| Upgrade | Stop, start the new image. Leases are re-announced; static entries, decisions and reputation are read back. |
