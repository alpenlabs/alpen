#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: wait-for-json-rpc.sh <rpc-url> [timeout-seconds] [interval-seconds]

Polls an Ethereum JSON-RPC endpoint with eth_blockNumber until it responds.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

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

    NOW_SECONDS="$(date +%s)"
    ELAPSED_SECONDS=$((NOW_SECONDS - START_SECONDS))
    if ((ELAPSED_SECONDS >= TIMEOUT_SECONDS)); then
        echo "JSON-RPC endpoint did not become ready within ${TIMEOUT_SECONDS}s: ${RPC_URL}" >&2
        exit 1
    fi

    sleep "${INTERVAL_SECONDS}"
done
