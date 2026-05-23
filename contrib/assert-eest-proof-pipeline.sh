#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: assert-eest-proof-pipeline.sh --rpc-endpoint URL [options]

Options:
  --target-block-number N  Block number that must be covered by a proof-ready chunk.
                           Defaults to eth_blockNumber at assertion start.
  --advanced-from N        Assert the chain advanced beyond this block number.
  --timeout SEC            Max wait for proof status. Default: 900.
  --poll SEC               Poll interval. Default: 5.
  -h, --help               Show this help.
EOF
}

RPC_ENDPOINT=""
TARGET_BLOCK_NUMBER=""
ADVANCED_FROM=""
TIMEOUT_SECONDS="900"
POLL_SECONDS="5"

while (($#)); do
    case "$1" in
        --rpc-endpoint)
            RPC_ENDPOINT="${2:?missing value for --rpc-endpoint}"
            shift 2
            ;;
        --target-block-number)
            TARGET_BLOCK_NUMBER="${2:?missing value for --target-block-number}"
            shift 2
            ;;
        --advanced-from)
            ADVANCED_FROM="${2:?missing value for --advanced-from}"
            shift 2
            ;;
        --timeout)
            TIMEOUT_SECONDS="${2:?missing value for --timeout}"
            shift 2
            ;;
        --poll)
            POLL_SECONDS="${2:?missing value for --poll}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

if [[ -z "${RPC_ENDPOINT}" ]]; then
    echo "--rpc-endpoint is required" >&2
    usage >&2
    exit 1
fi

rpc() {
    local method="$1"
    curl -sf -X POST "${RPC_ENDPOINT}" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":[],\"id\":1}"
}

block_number() {
    RPC_RESPONSE="$(rpc eth_blockNumber)" python3 - <<'PY'
import json
import os
import sys

response = json.loads(os.environ["RPC_RESPONSE"])
if "error" in response:
    print(response["error"], file=sys.stderr)
    sys.exit(1)
print(int(response["result"], 16))
PY
}

if [[ -z "${TARGET_BLOCK_NUMBER}" ]]; then
    TARGET_BLOCK_NUMBER="$(block_number)"
fi

if [[ ! "${TARGET_BLOCK_NUMBER}" =~ ^[0-9]+$ ]]; then
    echo "--target-block-number must be a decimal integer" >&2
    exit 1
fi

if [[ -n "${ADVANCED_FROM}" ]]; then
    if [[ ! "${ADVANCED_FROM}" =~ ^[0-9]+$ ]]; then
        echo "--advanced-from must be a decimal integer" >&2
        exit 1
    fi
    if ((TARGET_BLOCK_NUMBER <= ADVANCED_FROM)); then
        echo "EEST did not advance the chain: before=${ADVANCED_FROM} after=${TARGET_BLOCK_NUMBER}" >&2
        exit 1
    fi
fi

START_SECONDS="$(date +%s)"

while true; do
    STATUS_JSON="$(rpc alpen_getProofPipelineStatus)"

    if STATUS_JSON="${STATUS_JSON}" TARGET_BLOCK_NUMBER="${TARGET_BLOCK_NUMBER}" python3 - <<'PY'
import json
import os
import sys

target = int(os.environ["TARGET_BLOCK_NUMBER"])
response = json.loads(os.environ["STATUS_JSON"])
if "error" in response:
    print(response["error"], file=sys.stderr)
    sys.exit(2)

result = response["result"]
chunk = result.get("latestProofReadyChunk")
if chunk and chunk.get("lastBlockNumber") is not None and int(chunk["lastBlockNumber"]) >= target:
    print(
        "EEST proof pipeline covered target block "
        f"{target} with proof-ready chunk ending at {chunk['lastBlockNumber']}"
    )
    sys.exit(0)

latest = result.get("latestChunk")
ready = "none" if chunk is None else chunk.get("lastBlockNumber")
latest_status = "none" if latest is None else f"{latest.get('status')}@{latest.get('lastBlockNumber')}"
print(
    "waiting for proof-ready EEST chunk: "
    f"target={target} latest_ready={ready} latest_chunk={latest_status}"
)
sys.exit(1)
PY
    then
        exit 0
    fi

    NOW_SECONDS="$(date +%s)"
    ELAPSED_SECONDS=$((NOW_SECONDS - START_SECONDS))
    if ((ELAPSED_SECONDS >= TIMEOUT_SECONDS)); then
        echo "timed out waiting for proof-ready chunk covering block ${TARGET_BLOCK_NUMBER}" >&2
        echo "last proof pipeline status:" >&2
        echo "${STATUS_JSON}" >&2
        exit 1
    fi

    sleep "${POLL_SECONDS}"
done
