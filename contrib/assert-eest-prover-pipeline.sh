#!/usr/bin/env bash
set -euo pipefail

RPC_ENDPOINT=""
START_BLOCK=1
END_BLOCK=""
BITCOIN_RPC_URL=""
BITCOIN_WALLET=""
BITCOIN_BLOCKS_PER_POLL=4
TIMEOUT_SECONDS=900
POLL_SECONDS=5

die() {
    echo "$*" >&2
    exit 1
}

while (($#)); do
    case "$1" in
        --rpc-endpoint) RPC_ENDPOINT="${2:?}"; shift 2 ;;
        --start-block) START_BLOCK="${2:?}"; shift 2 ;;
        --end-block) END_BLOCK="${2:?}"; shift 2 ;;
        --bitcoin-rpc-url) BITCOIN_RPC_URL="${2:?}"; shift 2 ;;
        --bitcoin-wallet) BITCOIN_WALLET="${2:?}"; shift 2 ;;
        --bitcoin-blocks) BITCOIN_BLOCKS_PER_POLL="${2:?}"; shift 2 ;;
        --timeout) TIMEOUT_SECONDS="${2:?}"; shift 2 ;;
        --poll) POLL_SECONDS="${2:?}"; shift 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done

[[ -n "${RPC_ENDPOINT}" ]] || die "--rpc-endpoint is required"
[[ -n "${END_BLOCK}" ]] || die "--end-block is required"
[[ "${START_BLOCK}" =~ ^[0-9]+$ && "${END_BLOCK}" =~ ^[0-9]+$ ]] || die "block bounds must be decimal integers"
((START_BLOCK >= 1 && END_BLOCK >= START_BLOCK)) || die "invalid block range ${START_BLOCK}..${END_BLOCK}"
[[ "${BITCOIN_BLOCKS_PER_POLL}" =~ ^[0-9]+$ && "${BITCOIN_BLOCKS_PER_POLL}" -gt 0 ]] || die "--bitcoin-blocks must be positive"

post_json() {
    local url="$1" version="$2" method="$3" params="${4:-[]}" response
    for _ in {1..5}; do
        if response="$(
            curl -sf -X POST "${url}" \
                -H "Content-Type: application/json" \
                -d "{\"jsonrpc\":\"${version}\",\"method\":\"${method}\",\"params\":${params},\"id\":1}"
        )" && [[ -n "${response}" ]]; then
            printf '%s\n' "${response}"
            return 0
        fi
        sleep 1
    done
    echo "JSON-RPC request failed or returned an empty response: ${method}" >&2
    return 1
}

json_result() {
    python3 -c 'import json,sys
r=json.load(sys.stdin)
if r.get("error") is not None:
    print(r["error"], file=sys.stderr); sys.exit(1)
v=r.get("result")
if v is None:
    print("missing JSON-RPC result", file=sys.stderr); sys.exit(1)
print(json.dumps(v) if isinstance(v, (dict, list)) else v)'
}

btc_endpoint() {
    [[ -z "${BITCOIN_WALLET}" ]] && printf '%s\n' "${BITCOIN_RPC_URL}" && return
    printf '%s/wallet/%s\n' "${BITCOIN_RPC_URL%/}" "${BITCOIN_WALLET}"
}

mine_bitcoin_blocks() {
    [[ -n "${BITCOIN_RPC_URL}" ]] || return 0
    if [[ -z "${MINING_ADDRESS:-}" ]]; then
        MINING_ADDRESS="$(post_json "$(btc_endpoint)" "1.0" getnewaddress | json_result)"
    fi
    post_json "$(btc_endpoint)" "1.0" generatetoaddress \
        "[${BITCOIN_BLOCKS_PER_POLL},\"${MINING_ADDRESS}\"]" >/dev/null
}

coverage_summary() {
    python3 -c 'import json,sys
c=json.load(sys.stdin)
print("EEST chunk proof coverage: requested={}..{} covered={} first_uncovered={}".format(
    c["start_block"], c["end_block"], str(bool(c["covered"])).lower(), c.get("first_uncovered_block")))
sys.exit(0 if c["covered"] else 1)'
}

echo "waiting for EE chunk proofs to cover EEST block range ${START_BLOCK}..${END_BLOCK}"
START_SECONDS="$(date +%s)"

while true; do
    if COVERAGE="$(post_json "${RPC_ENDPOINT}" "2.0" alpen_getChunkProofCoverage "[${START_BLOCK},${END_BLOCK}]" | json_result)"; then
        if SUMMARY="$(printf '%s\n' "${COVERAGE}" | coverage_summary)"; then
            echo "${SUMMARY}"
            echo "EEST block range is covered by EE chunk proofs"
            exit 0
        fi
        echo "${SUMMARY}"
    else
        echo "unable to fetch EEST chunk proof coverage; retrying" >&2
    fi

    (( $(date +%s) - START_SECONDS < TIMEOUT_SECONDS )) \
        || die "timed out waiting for EE chunk proofs to cover EEST block range ${START_BLOCK}..${END_BLOCK}"
    mine_bitcoin_blocks || echo "unable to mine Bitcoin blocks while polling; retrying" >&2
    sleep "${POLL_SECONDS}"
done
