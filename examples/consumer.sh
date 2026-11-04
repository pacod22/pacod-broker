#!/usr/bin/env bash
# A consumer: discover, call what was found (here: pretend), report how it went and rate it.
#
#   BROKER=http://127.0.0.1:8060 examples/consumer.sh
#
# In open mode a client names itself with X-Broker-Client so that its reports count; with
# BROKER_AUTH=oidc set TOKEN to a pacod-id token instead.
set -euo pipefail
BROKER="${BROKER:-http://127.0.0.1:8060}"
if [ -n "${TOKEN:-}" ]; then WHO=(-H "Authorization: Bearer $TOKEN"); else WHO=(-H "X-Broker-Client: ${CLIENT:-demo-agent}"); fi

# The cheapest model priced in euros, with tools and at least 100k context, as priced at
# 23:00 UTC tonight. The broker converts no currency, so a price sort names one.
tonight="$(date -u +%Y-%m-%d)T23:00:00Z"
result=$(curl -fsS -X POST "$BROKER/v1/discover" "${WHO[@]}" -H 'content-type: application/json' -d "{
  \"kind\": \"model\",
  \"capabilities\": [\"tools\"],
  \"model\": {\"min_context_window\": 100000},
  \"at\": \"$tonight\",
  \"currency\": \"EUR\",
  \"sort\": \"price:input\",
  \"limit\": 3
}")
echo "$result" | python3 -c '
import json, sys
d = json.load(sys.stdin)
print(d["total"], "found")
for h in d["hits"]:
    p = h.get("price") or {}
    print("  %-32s %-12s input %s %s (%s)  why: %s" % (h["resource"]["name"], h["provider"]["name"], p.get("rates", {}).get("input"), p.get("currency", ""), p.get("source"), ", ".join(h["why"]["matched"])))
'
ticket=$(echo "$result" | python3 -c 'import json,sys; h=json.load(sys.stdin)["hits"]; print(h[0]["ticket"] if h else "")')
[ -z "$ticket" ] && exit 0

# How the call went, quoting the ticket; then a rating.
curl -fsS -X POST "$BROKER/v1/reports/usage" "${WHO[@]}" -H 'content-type: application/json' \
  -d "{\"reports\": [{\"ticket\": \"$ticket\", \"outcome\": \"ok\", \"ttft_ms\": 340, \"ms_per_token\": 19}]}"
echo
curl -fsS -X POST "$BROKER/v1/reports/feedback" "${WHO[@]}" -H 'content-type: application/json' \
  -d "{\"ticket\": \"$ticket\", \"rating\": 5, \"comment\": \"Fast and cheap at night\"}"
echo
