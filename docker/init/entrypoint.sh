#!/usr/bin/env bash
set -euo pipefail

# Init one-shot service entrypoint.
#
# Waits for a bitcoin node to be reachable and at a sufficient block height,
# then runs init-network.sh to generate keys, params, and the .env file.
# Exits on success so dependent services (strata, signer, alpen-client) can start.

BITCOIND_RPC_URL="${BITCOIND_RPC_URL:?BITCOIND_RPC_URL must be set}"
BITCOIND_RPC_USER="${BITCOIND_RPC_USER:?BITCOIND_RPC_USER must be set}"
BITCOIND_RPC_PASSWORD="${BITCOIND_RPC_PASSWORD:?BITCOIND_RPC_PASSWORD must be set}"
BITCOIN_NETWORK="${BITCOIN_NETWORK:-signet}"
GENESIS_L1_HEIGHT="${GENESIS_L1_HEIGHT:-0}"
DATATOOL_PATH="${DATATOOL_PATH:-/usr/local/bin/strata-datatool}"
OUTPUT_DIR="${OUTPUT_DIR:-/app/configs/generated}"

rpc_call() {
    curl -sf -u "${BITCOIND_RPC_USER}:${BITCOIND_RPC_PASSWORD}" \
        -d "{\"jsonrpc\":\"1.0\",\"method\":\"$1\",\"params\":$2}" \
        "${BITCOIND_RPC_URL}"
}

wait_for_bitcoin() {
    echo "waiting for bitcoin node at ${BITCOIND_RPC_URL}..."
    local attempt=0
    while true; do
        if result=$(rpc_call getblockchaininfo '[]' 2>/dev/null); then
            local height
            height=$(echo "${result}" | jq -r '.result.blocks')
            if [ "${height}" -ge "${GENESIS_L1_HEIGHT}" ]; then
                echo "bitcoin ready: height=${height} (required=${GENESIS_L1_HEIGHT})"
                return 0
            fi
            echo "bitcoin reachable but height=${height} < required=${GENESIS_L1_HEIGHT}, waiting..."
        else
            attempt=$((attempt + 1))
            if [ $((attempt % 10)) -eq 0 ]; then
                echo "still waiting for bitcoin node (attempt ${attempt})..."
            fi
        fi
        sleep 2
    done
}

wait_for_bitcoin

# Validate existing params against the current bitcoin chain.
# If the genesis block hash in params doesn't match the actual chain, wipe and regenerate.
validate_existing_params() {
    local l1_view="${OUTPUT_DIR}/genesis-l1-view.json"
    if [ ! -f "${l1_view}" ]; then
        return 0
    fi

    local params_height params_blkid
    params_height=$(jq -r '.blk.height' "${l1_view}" 2>/dev/null || echo "")
    params_blkid=$(jq -r '.blk.blkid' "${l1_view}" 2>/dev/null || echo "")

    if [ -z "${params_height}" ] || [ -z "${params_blkid}" ]; then
        echo "existing params have invalid L1 view, regenerating..."
        rm -rf "${OUTPUT_DIR}"
        return 0
    fi

    # Ask bitcoin for the block hash at that height
    local chain_hash
    chain_hash=$(rpc_call getblockhash "[${params_height}]" 2>/dev/null | jq -r '.result // empty' || true)

    if [ -z "${chain_hash}" ]; then
        echo "bitcoin does not have block at height ${params_height}, regenerating..."
        rm -rf "${OUTPUT_DIR}"
        return 0
    fi

    if [ "${chain_hash}" != "${params_blkid}" ]; then
        echo "chain mismatch at height ${params_height}: params=${params_blkid} chain=${chain_hash}"
        echo "wiping stale params and regenerating..."
        rm -rf "${OUTPUT_DIR}"
    else
        echo "existing params match current chain at height ${params_height}"
    fi
}

validate_existing_params

export BITCOIN_NETWORK
export BITCOIN_RPC_URL="${BITCOIND_RPC_URL}"
export BITCOIN_RPC_USER="${BITCOIND_RPC_USER}"
export BITCOIN_RPC_PASSWORD="${BITCOIND_RPC_PASSWORD}"
export GENESIS_L1_HEIGHT
export OUTPUT_DIR
export ENV_FILE="${OUTPUT_DIR}/.env.alpen"

exec /usr/local/bin/init-network.sh --sequencer "${DATATOOL_PATH}"
