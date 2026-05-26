#!/usr/bin/env bash
set -euo pipefail

RPC_URL="${1:?missing rpc url}"
TIMEOUT_SECONDS="${2:-120}"
INTERVAL_SECONDS="${3:-2}"
START_SECONDS="$(date +%s)"

while true; do
    if curl -sf -X POST "${RPC_URL}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        >/dev/null 2>&1; then
        echo "JSON-RPC endpoint is up: ${RPC_URL}"
        exit 0
    fi

    if (( $(date +%s) - START_SECONDS >= TIMEOUT_SECONDS )); then
        echo "JSON-RPC endpoint did not become ready within ${TIMEOUT_SECONDS}s: ${RPC_URL}" >&2
        exit 1
    fi

    sleep "${INTERVAL_SECONDS}"
done
