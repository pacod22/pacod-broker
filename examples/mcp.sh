#!/usr/bin/env bash
# The broker as an MCP server: list its tools, then discover through one.
set -euo pipefail
BROKER="${BROKER:-http://127.0.0.1:8060}"
call() { curl -fsS -X POST "$BROKER/mcp" -H 'content-type: application/json' -H 'X-Broker-Client: mcp-demo' -d "$1"; echo; }
call '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"demo","version":"1"}}}'
call '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
call '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"discover_resources","arguments":{"kind":"mcp_server","q":"weather"}}}'
