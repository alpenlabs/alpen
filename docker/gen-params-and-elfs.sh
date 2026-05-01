#!/usr/bin/env bash
set -euo pipefail

# Builds strata-datatool (and optionally SP1 ELFs), waits for bitcoin,
# validates params, and runs init-network.sh to generate keys + params.
#
# Called by `just docker-seq-up` before starting the compose stack.
# Reads configuration from .env in the docker/ directory.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Source .env — set -a exports all vars automatically
if [ -f "${SCRIPT_DIR}/.env" ]; then
    set -a
    # shellcheck source=.env.example
    . "${SCRIPT_DIR}/.env"
    set +a
fi

OUTPUT_DIR="${SCRIPT_DIR}/configs/generated"
ELF_DIR="${SCRIPT_DIR}/elfs"
DATATOOL_BIN="${REPO_ROOT}/target/release/strata-datatool"

# This script runs on the host, so replace docker container hostname with localhost.
BITCOIND_RPC_URL="${BITCOIND_RPC_URL//bitcoind/localhost}"

# JSON-RPC call to bitcoind using credentials from env.
rpc_call() {
    curl -sf -u "${BITCOIND_RPC_USER}:${BITCOIND_RPC_PASSWORD}" \
        -d "{\"jsonrpc\":\"1.0\",\"method\":\"$1\",\"params\":$2}" \
        "${BITCOIND_RPC_URL}"
}

# Blocks until bitcoind is reachable and has mined at least GENESIS_L1_HEIGHT blocks.
wait_for_bitcoin() {
    echo "waiting for bitcoin node at ${BITCOIND_RPC_URL}..."
    while true; do
        if result=$(rpc_call getblockchaininfo '[]' 2>/dev/null); then
            height=$(echo "${result}" | jq -r '.result.blocks')
            if [ "${height}" -ge "${GENESIS_L1_HEIGHT}" ]; then
                echo "bitcoin ready: height=${height} (L1 genesis height=${GENESIS_L1_HEIGHT})"
                return 0
            fi
            echo "bitcoin reachable but height=${height} < L1 genesis height=${GENESIS_L1_HEIGHT}, waiting..."
        fi
        sleep 2
    done
}

# Checks if existing params match the current bitcoin chain.
# Wipes and recreates OUTPUT_DIR if the genesis block hash doesn't match.
validate_params() {
    mkdir -p "${OUTPUT_DIR}"
    local l1_view="${OUTPUT_DIR}/genesis-l1-view.json"

    if [ ! -f "${l1_view}" ]; then
        return 0
    fi

    local params_height params_blkid
    params_height=$(jq -r '.blk.height' "${l1_view}" 2>/dev/null || echo "")
    params_blkid=$(jq -r '.blk.blkid' "${l1_view}" 2>/dev/null || echo "")

    if [ -z "${params_height}" ] || [ -z "${params_blkid}" ]; then
        echo "invalid L1 view, regenerating..."
        rm -rf "${OUTPUT_DIR}"
        mkdir -p "${OUTPUT_DIR}"
        return 0
    fi

    local chain_hash
    chain_hash=$(rpc_call getblockhash "[${params_height}]" 2>/dev/null | jq -r '.result // empty' || true)

    if [ -z "${chain_hash}" ] || [ "${chain_hash}" != "${params_blkid}" ]; then
        echo "stale params detected, regenerating..."
        rm -rf "${OUTPUT_DIR}"
        mkdir -p "${OUTPUT_DIR}"
    else
        echo "existing params match current chain at height ${params_height}"
    fi
}

# ---- Build datatool (and ELFs if DATATOOL_CARGO_FEATURES is set) ----

echo "building strata-datatool (fast if unchanged)..."
cd "${REPO_ROOT}"
cargo build --locked --release --bin strata-datatool \
    ${DATATOOL_CARGO_FEATURES:+--features "$DATATOOL_CARGO_FEATURES"}

# ---- Export SP1 ELFs if built ----

mkdir -p "${ELF_DIR}"
if [ -n "${DATATOOL_CARGO_FEATURES:-}" ]; then
    cp "${REPO_ROOT}"/provers/sp1/guest-*/cache/*.elf "${ELF_DIR}/"
    echo "exported SP1 ELFs to ${ELF_DIR}/"
fi

# ---- Wait for bitcoin, validate params, generate ----

wait_for_bitcoin
validate_params

export OUTPUT_DIR
export ENV_FILE="${OUTPUT_DIR}/.env.alpen"

"${SCRIPT_DIR}/init-network.sh" --sequencer "${DATATOOL_BIN}"
