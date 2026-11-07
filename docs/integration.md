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

The concept is [pacod-gw.md](pacod-gw.md): the roles the gateway can take towards the broker, scenarios, mappings and phases. All three phases are built (its section 11); the first changes nothing in gw's core:

| What | How |
|---|---|
| A bridge service | `crates/pacod-gw-broker` and service `gw-broker` (compose profile `broker`) in pacod-gw, with `pacod-broker-model`, `-index` and `-client` vendored (`scripts/vendor-pacod-broker.sh`). Each offering organisation has a pacod-id token holding `broker.provider` there; the bridge's own token holds `broker.consumer` and `broker.witness`. |
| Offerings | The bridge announces each organisation's offered templates (its file, or a template's `broker_offer`), with interface `pacod-gw` and the dialects, prices from gw's repository and live state from its breakers. |
| Prices | The broker serves a feed in gw's `pacod` format; gw's price tracker reads it as any feed. |
| Evidence | The bridge holds its own tickets for upstreams that are broker resources, and reports sampled usage from gw's analytics. From phase 2 it also reports calls to offerings on the consumers' own tickets, as a delegated witness (`reports.witness`). |
| A gw role | Phase 1 reads with a platform `viewer`; `broker_bridge` in `authz/gw.yaml` (phase 2) also reads the calls that quoted a ticket. |
| The core (phases 2 and 3) | `PGW_BROKER_URL` and `PGW_BROKER_TOKEN` (`broker.consumer`): proposals from the broker, pools by discovery, leased upstreams, `/mcp`, the console's Broker page. |

## 3. pacod-id

| What | Why |
|---|---|
| A `broker` brand | `crates/pacod-id/src/config.rs` hard-codes the brands, mirrored in `packages/ui/src/lib/brands.ts` and `tokens/brands.json`. Until a `broker` brand exists, the console uses the default look and sign-in pages show the realm's brand. |
| Organisation verification | The client `identity` component and provider `trust` would take a "verified organisation" flag from pacod-id if it had one. Until then broker admins verify providers themselves. |

## 4. pacod-ap: agents as resources, and the broker as a tool

pacod-ap is one installation without organisations, so it announces as one provider, `pacod-ap`, with one pacod-id token holding `broker.consumer` and `broker.provider` at the platform (`BROKER_TOKEN`).

| What | How |
|---|---|
| Agents offered | An agent's published version carries `meta.broker` (`platform` or `public`, and a summary), set in the agent editor. ap-admin-api announces each offered production agent as kind `agent`, interface `openai-chat` at `BROKER_AGENT_URL/v1`, and the agent's slug as `model.upstream_id`. Callers without an assignment may call an offered agent and nothing else. |
| Its MCP server | `POST /mcp` on ap-api (MCP streamable HTTP, JSON answers): `list_agents`, `list_facts` and `get_fact`, for offered agents only. It is announced as kind `mcp_server`, protocol `mcp-streamable-http`, at `BROKER_MCP_URL`. |
| The edge | `platform/caddy/ap-agents.caddy`: the site `AP_AGENT_API_SITE` routes `/v1/*` and `/mcp` to ap-api, which authenticates every call (pacod-id tokens, `pidt_` service tokens). pacod-demo serves it at `agents.<domain>`. |
| A search tool | `broker_search` runs `POST /v1/discover` during a turn and puts the hits into the context as data, never instructions. It is opt-in: only an agent whose tool list names it gets it. Without a broker it answers "unavailable" and the turn goes on. |
| The console | The Broker page (`/broker`) shows the status, what pacod-ap announces, and a search. The model pickers show a model's reputation where the gateway names its `broker_resource_id`. |
| The connection | `BROKER_URL`, `BROKER_TOKEN`, `BROKER_AGENT_URL`, `BROKER_MCP_URL` and the timings are the deployment's; a superuser may set each on the console's Broker page instead, with a connection test. A token goes only to the address it was given for: another broker address needs that broker's token (or none, in open mode). |
| Lifecycle | The announcer rereads every `BROKER_REFRESH_SECONDS`, updates under the same lease, heartbeats, and says goodbye when nothing is offered or the service stops. A connection changed on the Broker page ends the lease at the old broker and announces at the new one. Without a broker address nothing is announced. |

## 5. pacod-cms: websites as datasets

pacod-cms has organisations, so each one announces as itself, as pacod-gw's do: its members' calls never count towards its own reputation.

| What | How |
|---|---|
| Websites offered | A website's settings screen offers it to the organisation (`organisation:<id>`), the platform or the public, with a one-line summary. The studio announces each organisation's offered, active websites as kind `dataset` under the provider `cms-<organisation>`. Each website has key = the website id and the interfaces `other:website`, `other:llms-txt`, `other:llms-full-txt`, `other:atom` and `other:sitemap`, each only while the website's own file settings leave that file on. The robots AI switches become tags (`no-ai-training`, `no-ai-answering`). |
| Provider accounts | The studio's provisioner account holds `cms.provisioner` and the broker's `provisioner` role at the platform. With them it makes a service account `cms-broker` in an organisation when that organisation first offers a website, binds `broker.provider` there, and mints its token. The studio keeps the token in its control plane; a 401 makes it mint a new one. |
| The AI card | With the studio's consumer token (`PACOD_BROKER_TOKEN`, `broker.consumer`), the installation screen shows the reputation, price now and state of every gateway model that names its `broker_resource_id`. It also lists the chat models the broker has that answer a JSON schema and that the gateway does not offer yet. They are shown, never selectable: a feature calls only what the gateway offers. |
| Settings | `PACOD_BROKER_URL`, `PACOD_BROKER_TOKEN`, `PACOD_BROKER_OPEN_MODE` (a broker without tokens), `pacod.broker.refresh` and `pacod.broker.ttl`. All of it is off without the URL. |

## 6. The `provisioner` role

`broker.provisioner` (at the platform, permission `members.manage`, granted by `broker.manage`) lets one account give `broker.provider` at any organisation. pacod-id lets whoever holds the acting product's grant permission at a scope manage the service accounts there. Together, these let a product with organisations, such as pacod-cms, make one provider account per organisation without holding broker administration. It gives nothing else.

## 7. pacod-demo's credentials

The full stack's bootstrap (`scripts/bootstrap.py`) makes every token above:

| Token | Roles | For |
|---|---|---|
| `GWB_CONSUMER_TOKEN` | `broker.consumer`, `broker.witness` | pacod-gw's bridge |
| `PGW_BROKER_TOKEN` | `broker.consumer` | pacod-gw's core |
| `GWB_ORG_TOKEN_<ORG>` | `broker.provider` at the organisation | the bridge, per gateway organisation |
| `BROKER_AP_TOKEN` | `broker.consumer`, `broker.provider` | pacod-ap |
| `CMS_BROKER_TOKEN` | `broker.consumer` | pacod-cms's studio |
| (none) | `broker.provisioner` | added to pacod-cms's existing provisioner account |
