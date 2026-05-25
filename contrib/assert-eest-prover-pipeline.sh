#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: assert-eest-prover-pipeline.sh --rpc-endpoint URL [options]

Options:
  --rpc-endpoint URL       Execution JSON-RPC endpoint.
  --target-block-number N  Block number whose status must become confirmed/finalized.
                            Defaults to eth_blockNumber minus --target-depth
                            at assertion start.
  --target-depth N         Depth behind eth_blockNumber to target when
                            --target-block-number is not set. Default: 0.
  --advanced-from N        Assert the target block is greater than this block number.
  --min-confirmed-advance N
                            With --advanced-from, accept any confirmed/finalized
                            block in the range [advanced-from + N, target].
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
TARGET_DEPTH="0"
ADVANCED_FROM=""
MIN_CONFIRMED_ADVANCE=""
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
        --target-depth)
            TARGET_DEPTH="${2:?missing value for --target-depth}"
            shift 2
            ;;
        --advanced-from)
            ADVANCED_FROM="${2:?missing value for --advanced-from}"
            shift 2
            ;;
        --min-confirmed-advance)
            MIN_CONFIRMED_ADVANCE="${2:?missing value for --min-confirmed-advance}"
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

if [[ ! "${TARGET_DEPTH}" =~ ^[0-9]+$ ]]; then
    echo "--target-depth must be a decimal integer" >&2
    exit 1
fi

if [[ -n "${MIN_CONFIRMED_ADVANCE}" && ! "${MIN_CONFIRMED_ADVANCE}" =~ ^[0-9]+$ ]]; then
    echo "--min-confirmed-advance must be a decimal integer" >&2
    exit 1
fi

rpc() {
    local method="$1"
    local params="${2:-[]}"
    local attempt
    local response

    for ((attempt = 1; attempt <= 5; attempt++)); do
        if response="$(
            curl -sf -X POST "${RPC_ENDPOINT}" \
                -H "Content-Type: application/json" \
                -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}"
        )" && [[ -n "${response}" ]]; then
            printf '%s\n' "${response}"
            return 0
        fi
        sleep 1
    done

    echo "JSON-RPC request failed or returned an empty response: ${method}" >&2
    return 1
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
    local attempt
    local response

    for ((attempt = 1; attempt <= 5; attempt++)); do
        if response="$(
            curl -sf -X POST "$(bitcoin_rpc_endpoint)" \
                -H "Content-Type: application/json" \
                -d "{\"jsonrpc\":\"1.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}"
        )" && [[ -n "${response}" ]]; then
            printf '%s\n' "${response}"
            return 0
        fi
        sleep 1
    done

    echo "Bitcoin JSON-RPC request failed or returned an empty response: ${method}" >&2
    return 1
}

json_result() {
    RPC_RESPONSE="$1" python3 - <<'PY'
import json
import os
import sys

raw_response = os.environ["RPC_RESPONSE"].strip()
if not raw_response:
    print("empty JSON-RPC response", file=sys.stderr)
    sys.exit(1)
response = json.loads(raw_response)
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

raw_response = os.environ["RPC_RESPONSE"].strip()
if not raw_response:
    print("empty JSON-RPC response", file=sys.stderr)
    sys.exit(1)
response = json.loads(raw_response)
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

raw_response = os.environ["RPC_RESPONSE"].strip()
if not raw_response:
    print("empty JSON-RPC response", file=sys.stderr)
    sys.exit(1)
response = json.loads(raw_response)
if "error" in response and response["error"] is not None:
    print(response["error"], file=sys.stderr)
    sys.exit(1)
print(response["result"]["status"])
PY
}

is_confirmed_or_finalized() {
    local status="$1"
    [[ "${status}" == "confirmed" || "${status}" == "finalized" ]]
}

block_status_for_number() {
    local block_number="$1"
    local block_hash
    local status

    block_hash="$(block_hash_for_number "${block_number}")"
    status="$(block_status "${block_hash}")"
    printf '%s %s\n' "${block_hash}" "${status}"
}

find_confirmed_block_in_range() {
    local lower_block_number="$1"
    local upper_block_number="$2"
    local low="${lower_block_number}"
    local high="${upper_block_number}"
    local found_block_number=""
    local found_block_hash=""
    local found_status=""

    while ((low <= high)); do
        local mid=$(((low + high) / 2))
        local status_result
        local block_hash
        local status

        if ! status_result="$(block_status_for_number "${mid}")"; then
            return 2
        fi
        read -r block_hash status <<<"${status_result}"

        if is_confirmed_or_finalized "${status}"; then
            found_block_number="${mid}"
            found_block_hash="${block_hash}"
            found_status="${status}"
            low=$((mid + 1))
        else
            high=$((mid - 1))
        fi
    done

    if [[ -z "${found_block_number}" ]]; then
        return 1
    fi

    printf '%s %s %s\n' "${found_block_number}" "${found_block_hash}" "${found_status}"
}

mine_bitcoin_blocks() {
    if [[ -z "${BITCOIN_RPC_URL}" ]]; then
        return
    fi

    if [[ -z "${MINING_ADDRESS:-}" ]]; then
        local address_response
        if ! address_response="$(bitcoin_rpc getnewaddress)"; then
            return 1
        fi
        if ! MINING_ADDRESS="$(json_result "${address_response}")"; then
            return 1
        fi
    fi

    bitcoin_rpc generatetoaddress "[${BITCOIN_BLOCKS_PER_POLL},\"${MINING_ADDRESS}\"]" >/dev/null
}

if [[ -z "${TARGET_BLOCK_NUMBER}" ]]; then
    CURRENT_BLOCK_NUMBER="$(block_number)"
    if ((CURRENT_BLOCK_NUMBER > TARGET_DEPTH)); then
        TARGET_BLOCK_NUMBER=$((CURRENT_BLOCK_NUMBER - TARGET_DEPTH))
    else
        TARGET_BLOCK_NUMBER=0
    fi
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

LOWER_TARGET_BLOCK_NUMBER=""
if [[ -n "${MIN_CONFIRMED_ADVANCE}" ]]; then
    if [[ -z "${ADVANCED_FROM}" ]]; then
        echo "--min-confirmed-advance requires --advanced-from" >&2
        exit 1
    fi

    LOWER_TARGET_BLOCK_NUMBER=$((ADVANCED_FROM + MIN_CONFIRMED_ADVANCE))
    if ((TARGET_BLOCK_NUMBER < LOWER_TARGET_BLOCK_NUMBER)); then
        echo "EEST did not advance enough for confirmed status assertion: before=${ADVANCED_FROM} min_advance=${MIN_CONFIRMED_ADVANCE} target=${TARGET_BLOCK_NUMBER}" >&2
        exit 1
    fi

    echo "waiting for an EEST target block in [${LOWER_TARGET_BLOCK_NUMBER}, ${TARGET_BLOCK_NUMBER}] to become confirmed or finalized"
else
    TARGET_BLOCK_HASH="$(block_hash_for_number "${TARGET_BLOCK_NUMBER}")"
    echo "waiting for EEST target block ${TARGET_BLOCK_NUMBER} (${TARGET_BLOCK_HASH}) to become confirmed or finalized"
fi

START_SECONDS="$(date +%s)"

while true; do
    if [[ -n "${MIN_CONFIRMED_ADVANCE}" ]]; then
        if FOUND_BLOCK="$(find_confirmed_block_in_range "${LOWER_TARGET_BLOCK_NUMBER}" "${TARGET_BLOCK_NUMBER}")"; then
            read -r FOUND_BLOCK_NUMBER FOUND_BLOCK_HASH STATUS <<<"${FOUND_BLOCK}"
            echo "EEST target block status: number=${FOUND_BLOCK_NUMBER} hash=${FOUND_BLOCK_HASH} status=${STATUS}"
            echo "EEST-generated blocks reached externally confirmed EE/OL status"
            exit 0
        else
            FIND_STATUS=$?
            if ((FIND_STATUS == 2)); then
                echo "unable to fetch EEST target block range status; retrying" >&2
            else
                echo "EEST target block status: range=${LOWER_TARGET_BLOCK_NUMBER}..${TARGET_BLOCK_NUMBER} status=pending"
            fi
        fi
    elif STATUS="$(block_status "${TARGET_BLOCK_HASH}")"; then
        echo "EEST target block status: number=${TARGET_BLOCK_NUMBER} status=${STATUS}"

        if is_confirmed_or_finalized "${STATUS}"; then
            echo "EEST-generated blocks reached externally confirmed EE/OL status"
            exit 0
        fi
    else
        echo "unable to fetch EEST target block status; retrying" >&2
    fi

    NOW_SECONDS="$(date +%s)"
    ELAPSED_SECONDS=$((NOW_SECONDS - START_SECONDS))
    if ((ELAPSED_SECONDS >= TIMEOUT_SECONDS)); then
        echo "timed out waiting for EEST target block ${TARGET_BLOCK_NUMBER} to become confirmed/finalized" >&2
        exit 1
    fi

    if ! mine_bitcoin_blocks; then
        echo "unable to mine Bitcoin blocks while polling; retrying" >&2
    fi
    sleep "${POLL_SECONDS}"
done
