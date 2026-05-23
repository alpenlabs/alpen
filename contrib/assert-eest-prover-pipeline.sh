#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: assert-eest-prover-pipeline.sh --rpc-endpoint URL [options]

Options:
  --rpc-endpoint URL       Execution JSON-RPC endpoint.
  --target-block-number N  Block number whose status must become confirmed/finalized.
                            Defaults to eth_blockNumber at assertion start.
  --advanced-from N        Assert the target block is greater than this block number.
  --bitcoin-rpc-url URL    Optional Bitcoin RPC URL used to mine regtest blocks while polling.
  --bitcoin-wallet NAME    Optional wallet name appended as /wallet/NAME for Bitcoin RPC.
  --bitcoin-blocks N       Bitcoin blocks to mine per poll when --bitcoin-rpc-url is set.
                            Default: 4.
  --timeout SEC            Max wait for block status. Default: 900.
  --poll SEC               Poll interval. Default: 5.
  -h, --help               Show this help.
EOF
}

RPC_ENDPOINT=""
TARGET_BLOCK_NUMBER=""
ADVANCED_FROM=""
BITCOIN_RPC_URL=""
BITCOIN_WALLET=""
BITCOIN_BLOCKS_PER_POLL="4"
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
        --bitcoin-rpc-url)
            BITCOIN_RPC_URL="${2:?missing value for --bitcoin-rpc-url}"
            shift 2
            ;;
        --bitcoin-wallet)
            BITCOIN_WALLET="${2:?missing value for --bitcoin-wallet}"
            shift 2
            ;;
        --bitcoin-blocks)
            BITCOIN_BLOCKS_PER_POLL="${2:?missing value for --bitcoin-blocks}"
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

if [[ ! "${BITCOIN_BLOCKS_PER_POLL}" =~ ^[0-9]+$ ]] || ((BITCOIN_BLOCKS_PER_POLL < 1)); then
    echo "--bitcoin-blocks must be a positive decimal integer" >&2
    exit 1
fi

rpc() {
    local method="$1"
    local params="${2:-[]}"

    curl -sf -X POST "${RPC_ENDPOINT}" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}"
}

bitcoin_rpc_endpoint() {
    if [[ -z "${BITCOIN_WALLET}" ]]; then
        printf '%s\n' "${BITCOIN_RPC_URL}"
        return
    fi

    printf '%s/wallet/%s\n' "${BITCOIN_RPC_URL%/}" "${BITCOIN_WALLET}"
}

bitcoin_rpc() {
    local method="$1"
    local params="${2:-[]}"

    curl -sf -X POST "$(bitcoin_rpc_endpoint)" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"1.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}"
}

json_result() {
    RPC_RESPONSE="$1" python3 - <<'PY'
import json
import os
import sys

response = json.loads(os.environ["RPC_RESPONSE"])
if "error" in response and response["error"] is not None:
    print(response["error"], file=sys.stderr)
    sys.exit(1)
result = response.get("result")
if result is None:
    print("missing JSON-RPC result", file=sys.stderr)
    sys.exit(1)
if isinstance(result, (dict, list)):
    print(json.dumps(result))
else:
    print(result)
PY
}

block_number() {
    local response
    response="$(rpc eth_blockNumber)"
    RPC_RESPONSE="${response}" python3 - <<'PY'
import json
import os
import sys

response = json.loads(os.environ["RPC_RESPONSE"])
if "error" in response and response["error"] is not None:
    print(response["error"], file=sys.stderr)
    sys.exit(1)
print(int(response["result"], 16))
PY
}

block_by_number() {
    local block_number="$1"
    local block_hex
    printf -v block_hex '0x%x' "${block_number}"
    json_result "$(rpc eth_getBlockByNumber "[\"${block_hex}\",false]")"
}

block_hash_for_number() {
    BLOCK_JSON="$(block_by_number "$1")" python3 - <<'PY'
import json
import os
import sys

block = json.loads(os.environ["BLOCK_JSON"])
if block is None:
    print("block not found", file=sys.stderr)
    sys.exit(1)
print(block["hash"])
PY
}

block_status() {
    local block_hash="$1"
    local response
    response="$(rpc alpen_getBlockStatus "[\"${block_hash}\"]")"
    RPC_RESPONSE="${response}" python3 - <<'PY'
import json
import os
import sys

response = json.loads(os.environ["RPC_RESPONSE"])
if "error" in response and response["error"] is not None:
    print(response["error"], file=sys.stderr)
    sys.exit(1)
print(response["result"]["status"])
PY
}

mine_bitcoin_blocks() {
    if [[ -z "${BITCOIN_RPC_URL}" ]]; then
        return
    fi

    if [[ -z "${MINING_ADDRESS:-}" ]]; then
        MINING_ADDRESS="$(json_result "$(bitcoin_rpc getnewaddress)")"
    fi

    bitcoin_rpc generatetoaddress "[${BITCOIN_BLOCKS_PER_POLL},\"${MINING_ADDRESS}\"]" >/dev/null
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

TARGET_BLOCK_HASH="$(block_hash_for_number "${TARGET_BLOCK_NUMBER}")"
echo "waiting for EEST target block ${TARGET_BLOCK_NUMBER} (${TARGET_BLOCK_HASH}) to become confirmed or finalized"

START_SECONDS="$(date +%s)"

while true; do
    STATUS="$(block_status "${TARGET_BLOCK_HASH}")"
    echo "EEST target block status: number=${TARGET_BLOCK_NUMBER} status=${STATUS}"

    if [[ "${STATUS}" == "confirmed" || "${STATUS}" == "finalized" ]]; then
        echo "EEST-generated blocks reached externally confirmed EE/OL status"
        exit 0
    fi

    NOW_SECONDS="$(date +%s)"
    ELAPSED_SECONDS=$((NOW_SECONDS - START_SECONDS))
    if ((ELAPSED_SECONDS >= TIMEOUT_SECONDS)); then
        echo "timed out waiting for EEST target block ${TARGET_BLOCK_NUMBER} to become confirmed/finalized" >&2
        exit 1
    fi

    mine_bitcoin_blocks
    sleep "${POLL_SECONDS}"
done
