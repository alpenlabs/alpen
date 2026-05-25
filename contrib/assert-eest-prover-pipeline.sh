#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: assert-eest-prover-pipeline.sh --rpc-endpoint URL --end-block N [options]

Options:
  --rpc-endpoint URL       Execution JSON-RPC endpoint.
  --start-block N          First EEST EE block number to require proof coverage for.
                            Default: 1.
  --end-block N            Last EEST EE block number to require proof coverage for.
  --bitcoin-rpc-url URL    Optional Bitcoin RPC URL used to mine regtest blocks while polling.
  --bitcoin-wallet NAME    Optional wallet name appended as /wallet/NAME for Bitcoin RPC.
  --bitcoin-blocks N       Bitcoin blocks to mine per poll when --bitcoin-rpc-url is set.
                            Default: 4.
  --timeout SEC            Max wait for chunk proof coverage. Default: 900.
  --poll SEC               Poll interval. Default: 5.
  -h, --help               Show this help.
EOF
}

RPC_ENDPOINT=""
START_BLOCK="1"
END_BLOCK=""
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
        --start-block)
            START_BLOCK="${2:?missing value for --start-block}"
            shift 2
            ;;
        --end-block)
            END_BLOCK="${2:?missing value for --end-block}"
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

if [[ -z "${END_BLOCK}" ]]; then
    echo "--end-block is required" >&2
    usage >&2
    exit 1
fi

if [[ ! "${START_BLOCK}" =~ ^[0-9]+$ ]] || ((START_BLOCK < 1)); then
    echo "--start-block must be a positive decimal integer" >&2
    exit 1
fi

if [[ ! "${END_BLOCK}" =~ ^[0-9]+$ ]] || ((END_BLOCK < START_BLOCK)); then
    echo "--end-block must be a decimal integer greater than or equal to --start-block" >&2
    exit 1
fi

if [[ ! "${BITCOIN_BLOCKS_PER_POLL}" =~ ^[0-9]+$ ]] || ((BITCOIN_BLOCKS_PER_POLL < 1)); then
    echo "--bitcoin-blocks must be a positive decimal integer" >&2
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
    python3 -c '
import json
import sys

raw_response = sys.stdin.read().strip()
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
'
}

chunk_proof_coverage() {
    rpc alpen_getChunkProofCoverage "[${START_BLOCK},${END_BLOCK}]" | json_result
}

coverage_summary() {
    python3 -c '
import json
import sys

coverage = json.loads(sys.stdin.read())
ranges = coverage["ranges"]
ready = [r for r in ranges if r["status"] == "proof_ready"]
pending = [r for r in ranges if r["status"] != "proof_ready"]
covered = bool(coverage["covered"])
first_uncovered = coverage.get("first_uncovered_block")

print(
    "EEST chunk proof coverage: "
    "requested={}..{} "
    "covered={} "
    "first_uncovered={} "
    "proof_ready_ranges={} "
    "total_ranges={}".format(
        coverage["start_block"],
        coverage["end_block"],
        str(covered).lower(),
        first_uncovered,
        len(ready),
        len(ranges),
    )
)

if ready:
    last_ready = max(r["end_block"] for r in ready)
    print("latest proof-ready chunk ends at block {}".format(last_ready))

for chunk in pending[:5]:
    print(
        "pending chunk: "
        "idx={} "
        "range={}..{} "
        "status={}".format(
            chunk["chunk_index"],
            chunk["start_block"],
            chunk["end_block"],
            chunk["status"],
        )
    )

sys.exit(0 if covered else 1)
'
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
        if ! MINING_ADDRESS="$(printf '%s\n' "${address_response}" | json_result)"; then
            return 1
        fi
    fi

    bitcoin_rpc generatetoaddress "[${BITCOIN_BLOCKS_PER_POLL},\"${MINING_ADDRESS}\"]" >/dev/null
}

echo "waiting for EE chunk proofs to cover EEST block range ${START_BLOCK}..${END_BLOCK}"
START_SECONDS="$(date +%s)"

while true; do
    if COVERAGE="$(chunk_proof_coverage)"; then
        if SUMMARY="$(printf '%s\n' "${COVERAGE}" | coverage_summary)"; then
            echo "${SUMMARY}"
            echo "EEST block range is covered by EE chunk proofs"
            exit 0
        fi
        echo "${SUMMARY}"
    else
        echo "unable to fetch EEST chunk proof coverage; retrying" >&2
    fi

    NOW_SECONDS="$(date +%s)"
    ELAPSED_SECONDS=$((NOW_SECONDS - START_SECONDS))
    if ((ELAPSED_SECONDS >= TIMEOUT_SECONDS)); then
        echo "timed out waiting for EE chunk proofs to cover EEST block range ${START_BLOCK}..${END_BLOCK}" >&2
        exit 1
    fi

    if ! mine_bitcoin_blocks; then
        echo "unable to mine Bitcoin blocks while polling; retrying" >&2
    fi
    sleep "${POLL_SECONDS}"
done
