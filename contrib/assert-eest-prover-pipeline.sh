#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: assert-eest-prover-pipeline.sh [options]

Options:
  --baseline-file FILE          File containing byte offset captured before EEST.
  --log-file FILE               Alpen-client service.log to inspect.
  --docker-container NAME       Docker container whose logs should be inspected.
  --bitcoin-service-log FILE    bitcoind service.log to parse RPC settings from.
  --bitcoin-container NAME      Docker bitcoind container used for mining.
  --bitcoin-rpc-user USER       bitcoind RPC user.
  --bitcoin-rpc-password PASS   bitcoind RPC password.
  --bitcoin-rpc-port PORT       bitcoind RPC port for local mining.
  --bitcoin-rpc-wallet NAME     bitcoind wallet name. Default: default.
  --timeout SEC                 Max wait for all proof signals. Default: 900.
  --poll SEC                    Poll interval. Default: 5.
  --blocks-per-step N           Blocks mined per poll. Default: 4.
  -h, --help                    Show this help.
EOF
}

BASELINE_FILE=""
LOG_FILE=""
DOCKER_CONTAINER=""
BITCOIN_SERVICE_LOG=""
BITCOIN_CONTAINER=""
BITCOIN_RPC_USER=""
BITCOIN_RPC_PASSWORD=""
BITCOIN_RPC_PORT=""
BITCOIN_RPC_WALLET="default"
TIMEOUT_SECONDS="900"
POLL_SECONDS="5"
BLOCKS_PER_STEP="4"

while (($#)); do
    case "$1" in
        --baseline-file)
            BASELINE_FILE="${2:?missing value for --baseline-file}"
            shift 2
            ;;
        --log-file)
            LOG_FILE="${2:?missing value for --log-file}"
            shift 2
            ;;
        --docker-container)
            DOCKER_CONTAINER="${2:?missing value for --docker-container}"
            shift 2
            ;;
        --bitcoin-service-log)
            BITCOIN_SERVICE_LOG="${2:?missing value for --bitcoin-service-log}"
            shift 2
            ;;
        --bitcoin-container)
            BITCOIN_CONTAINER="${2:?missing value for --bitcoin-container}"
            shift 2
            ;;
        --bitcoin-rpc-user)
            BITCOIN_RPC_USER="${2:?missing value for --bitcoin-rpc-user}"
            shift 2
            ;;
        --bitcoin-rpc-password)
            BITCOIN_RPC_PASSWORD="${2:?missing value for --bitcoin-rpc-password}"
            shift 2
            ;;
        --bitcoin-rpc-port)
            BITCOIN_RPC_PORT="${2:?missing value for --bitcoin-rpc-port}"
            shift 2
            ;;
        --bitcoin-rpc-wallet)
            BITCOIN_RPC_WALLET="${2:?missing value for --bitcoin-rpc-wallet}"
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
        --blocks-per-step)
            BLOCKS_PER_STEP="${2:?missing value for --blocks-per-step}"
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

if [[ -z "${BASELINE_FILE}" ]]; then
    echo "--baseline-file is required" >&2
    exit 1
fi

if [[ -n "${LOG_FILE}" && -n "${DOCKER_CONTAINER}" ]]; then
    echo "pass only one of --log-file or --docker-container" >&2
    exit 1
fi

if [[ -z "${LOG_FILE}" && -z "${DOCKER_CONTAINER}" ]]; then
    echo "one of --log-file or --docker-container is required" >&2
    exit 1
fi

if [[ ! -f "${BASELINE_FILE}" ]]; then
    echo "missing baseline file: ${BASELINE_FILE}" >&2
    exit 1
fi

BASELINE_OFFSET="$(tr -d '[:space:]' < "${BASELINE_FILE}")"
if [[ ! "${BASELINE_OFFSET}" =~ ^[0-9]+$ ]]; then
    echo "invalid baseline offset in ${BASELINE_FILE}: ${BASELINE_OFFSET}" >&2
    exit 1
fi

if [[ -n "${BITCOIN_SERVICE_LOG}" ]]; then
    if [[ ! -f "${BITCOIN_SERVICE_LOG}" ]]; then
        echo "missing bitcoin service log: ${BITCOIN_SERVICE_LOG}" >&2
        exit 1
    fi
    BITCOIN_RPC_PORT="${BITCOIN_RPC_PORT:-$(grep -m1 -o -- "-rpcport=[0-9]*" "${BITCOIN_SERVICE_LOG}" | cut -d= -f2)}"
    BITCOIN_RPC_USER="${BITCOIN_RPC_USER:-$(grep -m1 -o -- "-rpcuser=[^', ]*" "${BITCOIN_SERVICE_LOG}" | cut -d= -f2)}"
    BITCOIN_RPC_PASSWORD="${BITCOIN_RPC_PASSWORD:-$(grep -m1 -o -- "-rpcpassword=[^', ]*" "${BITCOIN_SERVICE_LOG}" | cut -d= -f2)}"
fi

if [[ -z "${BITCOIN_CONTAINER}" ]]; then
    : "${BITCOIN_RPC_PORT:?missing --bitcoin-rpc-port or --bitcoin-service-log}"
fi
: "${BITCOIN_RPC_USER:?missing --bitcoin-rpc-user or --bitcoin-service-log}"
: "${BITCOIN_RPC_PASSWORD:?missing --bitcoin-rpc-password or --bitcoin-service-log}"

TMP_LOG="$(mktemp)"
TMP_FRAGMENT="$(mktemp)"
trap 'rm -f "${TMP_LOG}" "${TMP_FRAGMENT}"' EXIT

capture_log_since_baseline() {
    if [[ -n "${DOCKER_CONTAINER}" ]]; then
        docker logs "${DOCKER_CONTAINER}" >"${TMP_LOG}" 2>&1
    else
        cp "${LOG_FILE}" "${TMP_LOG}"
    fi

    tail -c "+$((BASELINE_OFFSET + 1))" "${TMP_LOG}"
}

mine_blocks() {
    local mine_address

    if [[ -n "${BITCOIN_CONTAINER}" ]]; then
        mine_address="$(
            docker exec "${BITCOIN_CONTAINER}" bitcoin-cli \
                -regtest \
                "-rpcuser=${BITCOIN_RPC_USER}" \
                "-rpcpassword=${BITCOIN_RPC_PASSWORD}" \
                "-rpcwallet=${BITCOIN_RPC_WALLET}" \
                getnewaddress
        )"
        docker exec "${BITCOIN_CONTAINER}" bitcoin-cli \
            -regtest \
            "-rpcuser=${BITCOIN_RPC_USER}" \
            "-rpcpassword=${BITCOIN_RPC_PASSWORD}" \
            "-rpcwallet=${BITCOIN_RPC_WALLET}" \
            generatetoaddress "${BLOCKS_PER_STEP}" "${mine_address}" >/dev/null
    else
        mine_address="$(
            bitcoin-cli \
                -regtest \
                "-rpcport=${BITCOIN_RPC_PORT}" \
                "-rpcuser=${BITCOIN_RPC_USER}" \
                "-rpcpassword=${BITCOIN_RPC_PASSWORD}" \
                "-rpcwallet=${BITCOIN_RPC_WALLET}" \
                getnewaddress
        )"
        bitcoin-cli \
            -regtest \
            "-rpcport=${BITCOIN_RPC_PORT}" \
            "-rpcuser=${BITCOIN_RPC_USER}" \
            "-rpcpassword=${BITCOIN_RPC_PASSWORD}" \
            "-rpcwallet=${BITCOIN_RPC_WALLET}" \
            generatetoaddress "${BLOCKS_PER_STEP}" "${mine_address}" >/dev/null
    fi
}

has_pattern() {
    local pattern="$1"
    capture_log_since_baseline >"${TMP_FRAGMENT}"
    grep -Eq "${pattern}" "${TMP_FRAGMENT}"
}

START_SECONDS="$(date +%s)"

while true; do
    witness=0
    chunk=0
    acct=0
    update=0

    if has_pattern "persisted chunk witness"; then
        witness=1
    fi
    if has_pattern "marking chunk as proof-ready"; then
        chunk=1
    fi
    if has_pattern "persisting batch acct proof"; then
        acct=1
    fi
    if has_pattern "Submitted update for batch|submitted snark update to OL"; then
        update=1
    fi

    echo "EEST proof signals: witness=${witness} chunk=${chunk} acct=${acct} update=${update}"

    if ((witness && chunk && acct && update)); then
        if has_pattern "retries exhausted|task died mid-Proving and retries exhausted"; then
            echo "observed permanent prover failure after EEST baseline" >&2
            exit 1
        fi
        echo "EEST-generated blocks reached the EE chunk/acct proof pipeline"
        exit 0
    fi

    NOW_SECONDS="$(date +%s)"
    ELAPSED_SECONDS=$((NOW_SECONDS - START_SECONDS))
    if ((ELAPSED_SECONDS >= TIMEOUT_SECONDS)); then
        echo "timed out waiting for EEST proof signals after ${TIMEOUT_SECONDS}s" >&2
        echo "last log tail after baseline:" >&2
        capture_log_since_baseline | tail -200 >&2
        exit 1
    fi

    mine_blocks
    sleep "${POLL_SECONDS}"
done
