# pacod-broker: joining the pacod family

The broker runs on its own. This page lists what joining the rest of the family takes, in the other repositories. None of it is done yet: each section is a checklist for a change in that repository.

## 1. pacod-demo: the full stack

pacod-demo loads every product from sibling directories named `pacod-<product>`, so this repository is checked out as `../pacod-broker`. pacod-benchmark was the last product to join, and `grep -rn bench` in pacod-demo shows each of the places below.

| # | File | Change |
|---|---|---|
| 1 | `compose.yml` | Include `../pacod-broker/deploy/docker-compose.yml` and `overlays/broker.yml` with `env_file: [.state/env/stack.env, .state/env/broker.env]` |
| 2 | `overlays/broker.yml` (new) | `ports: !reset []`; `RUST_LOG: ${DEMO_LOG_LEVEL}`, `BROKER_LOG_FORMAT: ${DEMO_LOG_FORMAT}`; `BROKER_AUTH: oidc`, `BROKER_ID_ISSUER`, `BROKER_ID_CLIENT_SECRET`; for the console `BROKER_UI_AUTH: oidc`, `BROKER_UI_CLIENT_SECRET`, `BROKER_UI_SESSION_SECRET`, `BROKER_CONSOLE_URL`. Template: `overlays/bench.yml`. |
| 3 | `platform.yml` | **pacod-caddy:** `BROKER_SITE`, `BROKER_API_SITE`, `BROKER_CADDY_LOG_FORMAT`; mount `../pacod-broker/platform/caddy/broker.caddy:/etc/caddy/sites/broker.caddy:ro`; aliases `broker.${DEMO_DOMAIN}` and `broker-api.${DEMO_DOMAIN}`.<br>**pacod-prometheus:** mount `../pacod-broker/platform/prometheus/broker.yml:/etc/prometheus/scrape.d/broker.yml:ro` and `../pacod-broker/deploy/prometheus/rules:/etc/prometheus/rules/broker:ro`.<br>**pacod-grafana:** mount `../pacod-broker/deploy/grafana/dashboards:/etc/grafana/dashboards/broker:ro`.<br>No database: no `pacod-db-init` change. |
| 4 | `config/prometheus.yml` | Add `/etc/prometheus/rules/broker/*.yml` to `rule_files` |
| 5 | `config/grafana/dashboards.yml` | A provider `pacod-broker` (folder and uid), path `/etc/grafana/dashboards/broker` |
| 6 | `config/pacod-id.toml` | A `service` client `pacod-broker-service` (`product = "broker"`, `audience = "pacod-broker"`, `secret_env = "BROKER_ID_SERVICE_SECRET"`); a `confidential` client `pacod-broker-ui` (`redirect_uris = ["${BROKER_ORIGIN}/auth/callback"]`, `post_logout_redirect_uris`, `initiate_login_uri`, `audience = "pacod-broker"`); `broker.admin` in `accounts_managed_by` |
| 7 | `scripts/render-env.sh` | `BROKER_ORIGIN` and `BROKER_SITE` (`url broker`), `BROKER_API_SITE` (`url broker-api`), `BROKER_CADDY_LOG_FORMAT`; `broker` and `broker-api` in both lists of taken subdomains; `broker.admin` in `PID_BOOTSTRAP_ADMIN_ROLES`; write `.state/env/broker.env` |
| 8 | `scripts/init-secrets.sh` | `need BROKER_ID_SERVICE_SECRET hex 32`, `need BROKER_UI_CLIENT_SECRET hex 32`, `need BROKER_UI_SESSION_SECRET hex 32` |
| 9 | `scripts/preflight.sh` | `pacod-broker` in the repository list; `broker` in the container-prefix pattern |
| 10 | `scripts/verify.sh` / `verify.py` | Expect `pacod-broker` and `broker-console`; reach `pacod-broker-admin:8061` on `pacod-admin-net` only; the console's `/auth/login` and `/auth/callback` |
| 11 | `Makefile` | The `urls` echo and the `hosts` subdomain loop |
| 12 | `home/index.html`, `home/llms.txt`, `scripts/shots.py` | A product card `#pacod-broker` (badge BR) with links to the console and the API |
| 13 | `demo.env.example`, `README.md`, `docs/NETWORK.md`, `docs/LOGGING.md` | The reserved subdomains, the address table, the network and log tables |

## 2. pacod-gw: the gateway as provider, source and witness

The concept is [pacod-gw.md](pacod-gw.md): the roles the gateway can take towards the broker, scenarios, mappings and phases. Its first phase changes nothing in gw's core:

| What | How |
|---|---|
| A bridge service | `crates/pacod-gw-broker` and service `gw-broker` in pacod-gw, with `pacod-broker-client` vendored. A pacod-id service account holds `broker.provider` in each organisation that offers, and `broker.consumer`. |
| Offerings | The bridge announces each organisation's chosen templates and models (interface `pacod-gw` and the dialects), with prices from gw's repository and live state from its breakers. |
| Prices | The broker serves a feed in gw's `pacod` format; gw's price tracker reads it as any feed. |
| Evidence | The bridge holds its own tickets for upstreams that are broker resources, and reports sampled usage from gw's analytics. A usage report counts only from the ticket's own client, so the gateway cannot report on its callers' tickets until the broker has a delegated witness (phase 2). |
| A gw role | `broker_bridge` in `authz/gw.yaml`: reading models, prices, templates, health and analytics. |

## 3. pacod-id

| What | Why |
|---|---|
| A `broker` brand | `crates/pacod-id/src/config.rs` hard-codes the brands, mirrored in `packages/ui/src/lib/brands.ts` and `tokens/brands.json`. Until a `broker` brand exists, the console uses the default look and sign-in pages show the realm's brand. |
| Organisation verification | The client `identity` component and provider `trust` would take a "verified organisation" flag from pacod-id if it had one. Until then broker admins verify providers themselves. |

## 4. pacod-ap

Agents built on pacod-ap can use the broker as an MCP server (`/mcp`, tools `discover_resources` and `describe_resource`) to find tools and models at run time, quoting tickets when they report back.
