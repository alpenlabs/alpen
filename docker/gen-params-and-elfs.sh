#!/usr/bin/env bash
set -euo pipefail

# Builds strata-datatool, generates local network artifacts, and optionally
# builds SP1 guest artifacts.
#
# Called by `just docker-seq-up` before starting the compose stack.
# Reads configuration from .env in the docker/ directory.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Source .env — set -a exports all vars automatically
if [ -f "${SCRIPT_DIR}/.env" ]; then
    set -a
    # .env path is dynamic — shellcheck can't follow it
    # shellcheck disable=SC1091
    . "${SCRIPT_DIR}/.env"
    set +a
fi

OUTPUT_DIR="${SCRIPT_DIR}/configs/generated"
ELF_DIR="${SCRIPT_DIR}/elfs"
PREDICATE_DIR="${SCRIPT_DIR}/predicates/sp1"
DATATOOL_BIN="${REPO_ROOT}/target/release/strata-datatool"
CHECKPOINT_PREDICATE="${CHECKPOINT_PREDICATE:-always-accept}"

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
    local l1_anchor="${OUTPUT_DIR}/l1-anchor.json"

    if [ ! -f "${l1_anchor}" ]; then
        return 0
    fi

    local params_height params_blkid
    params_height=$(jq -r '.block.height' "${l1_anchor}" 2>/dev/null || echo "")
    params_blkid=$(jq -r '.block.blkid' "${l1_anchor}" 2>/dev/null || echo "")

    if [ -z "${params_height}" ] || [ -z "${params_blkid}" ]; then
        echo "invalid L1 anchor, regenerating..."
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

build_sp1_guest_artifacts() {
    echo "building SP1 guest artifacts (fast if unchanged)..."
    CHECKPOINT_RUNTIME_PARAMS_PATH="${OUTPUT_DIR}/ol-params.json" \
        cargo build --locked --release -p strata-sp1-guest-builder --features build-elf

    mkdir -p "${ELF_DIR}"
    cp "${REPO_ROOT}"/provers/sp1/guest-*/cache/*.elf "${ELF_DIR}/"
    cp "${REPO_ROOT}"/provers/sp1/guest-*/cache/*.artifact-manifest.json "${ELF_DIR}/"
    cp "${REPO_ROOT}"/provers/sp1/guest-*/cache/*.predicate "${PREDICATE_DIR}/"
    echo "exported SP1 ELFs to ${ELF_DIR}/"
    echo "exported SP1 artifact manifests to ${ELF_DIR}/"
    echo "exported SP1 predicates to ${PREDICATE_DIR}/"
}

prepare_sp1_checkpoint_predicate() {
    build_sp1_guest_artifacts
    CHECKPOINT_PREDICATE_FILE="${PREDICATE_DIR}/guest-checkpoint.predicate"
}

prepare_checkpoint_predicate() {
    mkdir -p "${PREDICATE_DIR}"

    case "${CHECKPOINT_PREDICATE}" in
        always-accept)
            CHECKPOINT_PREDICATE_FILE="${PREDICATE_DIR}/checkpoint-dev-empty.predicate"
            printf 'AlwaysAccept\n' > "${CHECKPOINT_PREDICATE_FILE}"
            echo "using dev-empty checkpoint predicate at ${CHECKPOINT_PREDICATE_FILE}"
            ;;
        sp1-groth16)
            prepare_sp1_checkpoint_predicate
            ;;
        bip340-schnorr-test)
            CHECKPOINT_PREDICATE_FILE="${REPO_ROOT}/functional-tests/fixtures/predicates/checkpoint-bip340-schnorr-test.predicate"
            echo "using test checkpoint predicate at ${CHECKPOINT_PREDICATE_FILE}"
            ;;
        *)
            echo "error: unsupported CHECKPOINT_PREDICATE=${CHECKPOINT_PREDICATE} (use always-accept, sp1-groth16, or bip340-schnorr-test)" >&2
            exit 1
            ;;
    esac
}

# ---- Build datatool and prepare checkpoint predicate metadata ----

echo "building strata-datatool (fast if unchanged)..."
cd "${REPO_ROOT}"
cargo build --locked --release --bin strata-datatool

# ---- Wait for bitcoin, validate params, generate base network artifacts ----

wait_for_bitcoin
validate_params

export OUTPUT_DIR

"${SCRIPT_DIR}/init-network.sh" --sequencer --base-only "${DATATOOL_BIN}"

prepare_checkpoint_predicate

# ---- Generate ASM params with the selected checkpoint predicate ----

rm -f "${OUTPUT_DIR}/asm-params.json"

"${SCRIPT_DIR}/init-network.sh" \
    --sequencer \
    --asm-only \
    --checkpoint-predicate-file "${CHECKPOINT_PREDICATE_FILE}" \
    "${DATATOOL_BIN}"
