#!/usr/bin/env bash
# A provider in a few lines: announce, keep the lease alive with heartbeats, say goodbye on exit.
#
#   BROKER=http://127.0.0.1:8060 examples/provider.sh
#
# With BROKER_AUTH=oidc, set TOKEN to the provider's pacod-id service-account token (pidt_…).
set -euo pipefail
BROKER="${BROKER:-http://127.0.0.1:8060}"
AUTH=()
[ -n "${TOKEN:-}" ] && AUTH=(-H "Authorization: Bearer $TOKEN")
NAME="${NAME:-local-ollama}"

ANNOUNCEMENT=$(cat <<JSON
{
  "ttl_s": 30,
  "provider": {
    "name": "$NAME",
    "summary": "Models served by Ollama on a workstation",
    "inventory": {
      "visibility": "platform",
      "hardware": [{"id": "gpu", "type": "gpu", "vendor": "nvidia", "model": "RTX-4090", "count": 1, "memory_gb": 24}],
      "software": [{"id": "ollama", "kind": "serving_engine", "name": "ollama", "version": "0.5.7"}]
    }
  },
  "resources": [{
    "key": "qwen",
    "kind": "model",
    "name": "qwen2.5-coder-32b",
    "summary": "Qwen 2.5 Coder 32B, q4_k_m, on one RTX 4090",
    "interfaces": [{"protocol": "openai-chat", "url": "http://workstation.lan:11434/v1"}],
    "model": {"family": "qwen", "context_window": 32768, "quantisation": "q4_k_m", "parameters_b": 32},
    "capabilities": {"tools": true, "streaming": true},
    "runs_on": {"hardware": ["gpu"], "software": ["ollama"]},
    "pricing": {"free": true},
    "availability": {"capacity": {"max_concurrency": 2}, "latency": {"ttft_ms": 450}},
    "access": {"visibility": "platform"}
  }]
}
JSON
)

answer=$(curl -fsS -X PUT "$BROKER/v1/announce" "${AUTH[@]}" -H 'content-type: application/json' -d "$ANNOUNCEMENT")
lease=$(printf '%s' "$answer" | sed -n 's/.*"lease_id":"\([^"]*\)".*/\1/p')
every=$(printf '%s' "$answer" | sed -n 's/.*"heartbeat_every_s":\([0-9]*\).*/\1/p')
echo "announced under $lease; heartbeat every ${every}s (Ctrl-C ends the lease)"

trap 'curl -fsS -X DELETE "$BROKER/v1/leases/$lease?provider=$NAME" "${AUTH[@]}" >/dev/null && echo "lease ended"; exit 0' INT TERM
while sleep "$every"; do
  # Report the live state with each heartbeat: status, concurrency in use, observed latency.
  if ! curl -fsS -X POST "$BROKER/v1/leases/$lease/heartbeat?provider=$NAME" "${AUTH[@]}" \
    -H 'content-type: application/json' -d '{"status": "active", "in_use": 0}' >/dev/null; then
    echo "the lease is gone (the broker restarted?); announce again" >&2
    exec "$0"
  fi
done
