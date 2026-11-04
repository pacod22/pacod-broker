# pacod-broker: every command a developer or operator needs. `make help` lists them.
SHELL := /bin/bash
.DEFAULT_GOAL := help
CRATES := -p pacod-broker -p pacod-broker-client -p pacod-broker-index -p pacod-broker-logging -p pacod-broker-model -p pacod-broker-reputation -p pacod-broker-suite
COMPOSE := docker compose -f deploy/docker-compose.yml
VERSION ?= 0.1.0
DEV_ENV := BROKER_LISTEN=127.0.0.1:18060 BROKER_ADMIN_LISTEN=127.0.0.1:18061 BROKER_METRICS_LISTEN=127.0.0.1:19160 BROKER_LOG_FORMAT=text

.PHONY: help
help: ## This list
	@grep -hE '^[a-z][a-z0-9-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-18s %s\n", $$1, $$2}'

# --- Checks -------------------------------------------------------------------------------
.PHONY: verify fmt fmt-check clippy test openapi openapi-check authz authz-check ui-check deny bench
verify: fmt-check clippy test openapi-check authz-check ui-check clients-test ## Every check before a commit

fmt: ## Format the Rust and console code (never the vendored crates)
	cargo fmt $(CRATES)
	cd ui && npx prettier --write src

fmt-check:
	cargo fmt $(CRATES) --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test: ## Rust tests: no database, no network
	cargo test --workspace

openapi: ## Write docs/openapi.json, the console's and the TypeScript client's types from the code
	UPDATE_OPENAPI=1 cargo test -p pacod-broker openapi
	cd ui && npm run api:types
	cd clients/typescript && npm run types

openapi-check: ## The committed OpenAPI document and types match the code
	cargo test -p pacod-broker openapi
	cd ui && npm run api:types >/dev/null && git diff --exit-code -- src/lib/api/schema.d.ts
	cd clients/typescript && npm run types >/dev/null && git diff --exit-code -- src/schema.d.ts

authz: ## Generate the console's authz.generated.ts from authz/broker.yaml
	cd ui && npm run authz:gen

authz-check:
	cd ui && npm run authz:gen >/dev/null && git diff --exit-code -- src/lib/authz.generated.ts

ui-check: ## Console: types, unit tests, build
	cd ui && npm run check && npm test && npm run build

deny: ## Licences, sources and advisories (cargo install cargo-deny --locked)
	cargo deny --locked check

bench: ## Discovery at 10 000 resources
	cargo bench -p pacod-broker-index --bench discover

# --- The scenario and load suite (docs/testing.md) -------------------------------------------
.PHONY: suite suite-load suite-one
SUITE := cargo run -q --release -p pacod-broker-suite -- --broker target/release/pacod-broker
SCALE ?= 1

suite: ## Every scenario against the release broker; report in target/suite
	cargo build -q --release -p pacod-broker
	$(SUITE)

suite-load: ## The load runs (SCALE=0.2 for a fifth of their size); report in target/suite
	cargo build -q --release -p pacod-broker
	$(SUITE) --load --scale $(SCALE)

suite-one: ## The scenarios or load runs whose name contains S (make suite-one S=lifecycle)
	@test -n "$(S)" || { echo "S=<name> is needed"; exit 1; }
	cargo build -q --release -p pacod-broker
	$(SUITE) --all --only $(S) --scale $(SCALE)

# --- Clients (docs/clients.md) ---------------------------------------------------------------
.PHONY: clients clients-test
clients: ## Install the Python client in clients/python/.venv and build the TypeScript one
	cd clients/python && uv venv -q .venv && uv pip install -q --python .venv/bin/python -e . pytest pytest-asyncio
	cd clients/typescript && npm ci --no-audit --no-fund && npm run build

clients-test: ## The three clients against a real broker (the Rust one runs in `make test`)
	cargo build -p pacod-broker
	cd clients/python && .venv/bin/pytest -q
	cd clients/typescript && npm run check && npm test

# --- Running ------------------------------------------------------------------------------
.PHONY: run run-catalog console network images up dev-up down logs smoke
run: ## The broker from the working tree: open mode, admin API on 127.0.0.1:18061
	$(DEV_ENV) cargo run -p pacod-broker -- serve

run-catalog: ## The same, with the example catalogue
	$(DEV_ENV) BROKER_CATALOG_FILE=examples/catalog.example.yaml cargo run -p pacod-broker -- serve

console: ## The console in development mode, docked to `make run` (http://127.0.0.1:5173)
	cd ui && BROKER_CONSOLE_BROKERS=default=http://127.0.0.1:18061 npm run dev

network: ## The shared networks (pacod-demo/docs/NETWORK.md); pacod-admin-net is internal
	@docker network inspect pacod-net >/dev/null 2>&1 || docker network create pacod-net >/dev/null
	@docker network inspect pacod-admin-net >/dev/null 2>&1 || docker network create --internal pacod-admin-net >/dev/null
	@test "$$(docker network inspect -f '{{.Internal}}' pacod-admin-net)" = true || { echo "pacod-admin-net must be internal"; exit 1; }

images: ## Build both images
	docker build -f deploy/docker/rust.Dockerfile --target broker --build-arg VERSION=$(VERSION) -t pacod-broker:$(VERSION) .
	docker build -f deploy/docker/ui.Dockerfile --target ui --build-arg VERSION=$(VERSION) -t pacod-broker-console:$(VERSION) .

up: network ## The broker and the console with their own edge and monitoring
	$(COMPOSE) -f deploy/docker-compose.standalone.yml up -d --wait

dev-up: network ## From the working tree: ports on 127.0.0.1, the example catalogue loaded
	$(COMPOSE) -f deploy/docker-compose.dev.yml up -d --build --wait

down: ## Stop whatever of the above runs
	$(COMPOSE) -f deploy/docker-compose.dev.yml -f deploy/docker-compose.standalone.yml down

logs:
	$(COMPOSE) logs -f pacod-broker

smoke: ## End to end against a running broker (BROKER and ADMIN default to the dev ports)
	scripts/smoke.sh

# --- pacod-id -----------------------------------------------------------------------------
.PHONY: vendor-pacod-id
vendor-pacod-id: ## Copy pacod-id's crates and npm packages from ../pacod-id
	scripts/vendor-pacod-id.sh

# --- Deployment files ---------------------------------------------------------------------
.PHONY: deploy-check
deploy-check: ## Compose files, the edge, the alert rules and the dashboard are valid
	$(COMPOSE) config -q
	$(COMPOSE) -f deploy/docker-compose.dev.yml config -q
	$(COMPOSE) -f deploy/docker-compose.standalone.yml config -q
	docker run --rm -v "$$PWD/deploy/prometheus/rules:/rules:ro" --entrypoint promtool prom/prometheus:v3.15.0 check rules /rules/broker.rules.yml
	docker run --rm -v "$$PWD/deploy/caddy/Caddyfile:/etc/caddy/Caddyfile:ro" -v "$$PWD/platform/caddy:/etc/caddy/sites:ro" caddy:2.11.6-alpine caddy validate --config /etc/caddy/Caddyfile
	python3 -c 'import json; json.load(open("deploy/grafana/dashboards/broker-overview.json"))'
