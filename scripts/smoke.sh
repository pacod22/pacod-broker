#!/usr/bin/env bash
# End to end against a running broker in open mode: a provider announces with an inventory and
# a price that changes at night; a consumer finds it, the provider heartbeats, the consumer
# reports usage and a rating with its ticket (and someone else cannot), MCP answers, the admin
# API sees the lease, and without heartbeats the lease runs out. Exits non-zero at the first
# step that does not do what it should.
#
#   BROKER=http://127.0.0.1:18060 ADMIN=http://127.0.0.1:18061 scripts/smoke.sh
set -euo pipefail
BROKER="${BROKER:-http://127.0.0.1:18060}"
ADMIN="${ADMIN:-http://127.0.0.1:18061}"
J=(-H 'content-type: application/json')
ME=(-H 'X-Broker-Client: smoke-client')

step() { printf '%-56s' "$1"; }
ok() { echo "ok${1:+ ($1)}"; }
fail() {
  echo "FAILED: $*"
  exit 1
}
py() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

step "the broker answers"
curl -fsS "$BROKER/healthz" >/dev/null || fail "no answer at $BROKER"
curl -fsS "$BROKER/readyz" >/dev/null || fail "not ready"
ok

step "a provider announces under a 6 s lease"
A=$(curl -fsS -X PUT "$BROKER/v1/announce" "${J[@]}" -d '{
  "ttl_s": 6,
  "provider": {"name": "smoke-gpu", "summary": "Smoke test",
    "inventory": {"visibility": "public",
      "hardware": [{"id": "g", "type": "gpu", "model": "H100-SXM", "count": 8, "memory_gb": 80}],
      "software": [{"id": "v", "kind": "serving_engine", "name": "vllm", "version": "0.7.3"}]}},
  "resources": [{"key": "m", "kind": "model", "name": "smoke-model", "summary": "A model for the smoke test",
    "interfaces": [{"protocol": "openai-chat", "url": "https://smoke.example/v1"}],
    "runs_on": {"hardware": ["g"], "software": ["v"]},
    "pricing": {"currency": "EUR", "timezone": "UTC", "periods": [{"valid_from": "2026-01-01T00:00:00Z",
      "rates": {"input": 1.0},
      "windows": [{"name": "night", "from_time": "22:00", "to_time": "06:00", "rates": {"input": 0.4}}]}]},
    "availability": {"latency": {"ttft_ms": 200}},
    "access": {"visibility": "public"}}]}')
LEASE=$(echo "$A" | py 'd["lease_id"]')
RID=$(echo "$A" | py 'd["resources"]["m"]')
ok "$LEASE"
sleep 0.3

step "it is found by hardware, software and night price"
D=$(curl -fsS -X POST "$BROKER/v1/discover" "${J[@]}" "${ME[@]}" -d '{"q": "smoke",
  "hardware": {"gpu_model": "H100-SXM"}, "software": [{"name": "vllm", "version": ">=0.7"}],
  "at": "2026-11-04T23:00:00Z"}')
[ "$(echo "$D" | py 'd["total"]')" = 1 ] || fail "$D"
[ "$(echo "$D" | py 'd["hits"][0]["price"]["rates"]["input"]')" = 0.4 ] || fail "night price: $D"
TICKET=$(echo "$D" | py 'd["hits"][0]["ticket"]')
ok

step "a heartbeat keeps the lease and reports live state"
curl -fsS -X POST "$BROKER/v1/leases/$LEASE/heartbeat" "${J[@]}" -d '{"status": "active", "in_use": 3}' >/dev/null
[ "$(curl -fsS "$BROKER/v1/resources/$RID" "${ME[@]}" | py 'd["live"]["in_use"]')" = 3 ] || fail "live state"
ok

step "usage and a rating count with the ticket"
U=$(curl -fsS -X POST "$BROKER/v1/reports/usage" "${J[@]}" "${ME[@]}" \
  -d "{\"reports\": [{\"ticket\": \"$TICKET\", \"outcome\": \"ok\", \"ttft_ms\": 190}]}")
[ "$(echo "$U" | py 'd["accepted"]')" = 1 ] || fail "$U"
F=$(curl -fsS -X POST "$BROKER/v1/reports/feedback" "${J[@]}" "${ME[@]}" -d "{\"ticket\": \"$TICKET\", \"rating\": 5}")
[ "$(echo "$F" | py 'd["accepted"]')" = 1 ] || fail "$F"
ok

step "another client cannot use the ticket"
X=$(curl -sS -X POST "$BROKER/v1/reports/feedback" "${J[@]}" -H 'X-Broker-Client: someone-else' \
  -d "{\"ticket\": \"$TICKET\", \"rating\": 1}")
[ "$(echo "$X" | py 'd["status"]')" = 422 ] || fail "$X"
ok

step "the broker answers through MCP"
M=$(curl -fsS -X POST "$BROKER/mcp" "${J[@]}" "${ME[@]}" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"discover_resources","arguments":{"q":"smoke"}}}')
[ "$(echo "$M" | py 'd["result"]["structuredContent"]["total"]')" = 1 ] || fail "$M"
ok

if curl -fsS "$ADMIN/api/v1/status" >/dev/null 2>&1; then
  step "the admin API sees the lease"
  N=$(curl -fsS "$ADMIN/api/v1/leases" | py "len([l for l in d if l['id'] == '$LEASE'])")
  [ "$N" = 1 ] || fail "lease not listed"
  ok
fi

step "without heartbeats the lease runs out"
sleep 8
[ "$(curl -fsS -X POST "$BROKER/v1/discover" "${J[@]}" "${ME[@]}" -d '{"q": "smoke"}' | py 'd["total"]')" = 0 ] ||
  fail "still found"
ok
echo "smoke test passed"
