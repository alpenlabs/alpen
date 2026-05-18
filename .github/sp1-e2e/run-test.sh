#!/usr/bin/env bash
set -euo pipefail

# SP1 E2E Test Runner
#
# Runs the full sequencer stack with real SP1 Groth16 proofs and validates
# checkpoint + EE batch proof acceptance. Reuses init-network.sh for params.
#
# Required env vars:
#   ECR_REGISTRY, IMAGE_TAG, NETWORK_PRIVATE_KEY

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DOCKER_DIR="${REPO_ROOT}/docker"
CHECKPOINT_TIMEOUT="${CHECKPOINT_TIMEOUT:-300}"
BATCH_TIMEOUT="${BATCH_TIMEOUT:-600}"

DATATOOL_IMAGE="${ECR_REGISTRY}/strata/strata-datatool:${IMAGE_TAG}"

ALPEN_ACCOUNT_ID="0101010101010101010101010101010101010101010101010101010101010101"
CHAIN_STATUS_PAYLOAD='{"jsonrpc":"2.0","method":"strata_getChainStatus","params":[],"id":1}'
SNARK_STATE_PAYLOAD='{"jsonrpc":"2.0","method":"strata_getSnarkAccountState","params":["'"${ALPEN_ACCOUNT_ID}"'","latest"],"id":1}'

# Signet config
export MNEMONIC="abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
export MINERENABLED=1
export MINE_GENESIS=1
export BITCOIND_RPC_USER=bitcoin
export BITCOIND_RPC_PASSWORD=bitcoin
export BITCOIN_NETWORK=signet
export BITCOIND_RPC_URL=http://localhost:38332
export CHECKPOINT_PREDICATE=sp1-groth16
export ALPEN_PREDICATE=sp1-groth16
export ALPEN_CHAIN_CONFIG="${REPO_ROOT}/crates/reth/chainspec/src/res/testnet-chain.json"

# --- Low-level helpers ---

cleanup() {
    echo "=== Collecting logs ==="
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" logs > "${SCRIPT_DIR}/e2e-logs.txt" 2>&1 || true
    docker compose -f "${DOCKER_DIR}/compose-signet.yml" logs >> "${SCRIPT_DIR}/e2e-logs.txt" 2>&1 || true
    echo "=== Tearing down ==="
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" down -v 2>/dev/null || true
    docker compose -f "${DOCKER_DIR}/compose-signet.yml" down -v 2>/dev/null || true
}
trap cleanup EXIT

json_rpc() {
    local payload="$1"
    local port="$2"

    curl -sf \
        -X POST \
        -H 'Content-Type: application/json' \
        -d "${payload}" \
        "http://localhost:${port}"
}

ol_rpc() { json_rpc "$1" 8432; }

wait_for_service() {
    local label="$1"
    local port="$2"
    local method="$3"
    local timeout="${4:-120}"

    echo "Waiting for ${label}..."
    local deadline=$((SECONDS + timeout))
    while [ $SECONDS -lt $deadline ]; do
        local payload='{"jsonrpc":"2.0","method":"'"${method}"'","params":[],"id":1}'
        if json_rpc "${payload}" "${port}" >/dev/null 2>&1; then
            echo "${label} ready"
            return 0
        fi
        sleep 2
    done
    echo "FAIL: ${label} not reachable within ${timeout}s"
    return 1
}

wait_for_strata()       { wait_for_service "strata"       8432 "strata_protocolVersion"; }
wait_for_alpen_client() { wait_for_service "alpen-client" 8545 "eth_blockNumber"; }

btc_height() {
    docker exec docker-bitcoind-1 bitcoin-cli -signet getblockcount 2>/dev/null || echo 0
}

parse_result() {
    python3 -c "import json,sys; print(json.load(sys.stdin)['result']$1)" 2>/dev/null
}

get_confirmed_epoch() {
    ol_rpc "${CHAIN_STATUS_PAYLOAD}" | parse_result "['confirmed']['epoch']" || echo 0
}

get_snark_seq_no() {
    ol_rpc "${SNARK_STATE_PAYLOAD}" | parse_result ".get('seq_no',0)" || echo 0
}

get_snark_update_vk() {
    ol_rpc "${SNARK_STATE_PAYLOAD}" | parse_result "['update_vk']" || echo "unknown"
}

# --- Step functions ---

start_signet_fast() {
    echo "=== Starting signet (BLOCKPRODUCTIONDELAY=1) ==="
    cd "${DOCKER_DIR}"
    export BLOCKPRODUCTIONDELAY=1
    docker compose -f compose-signet.yml up -d

    echo "Waiting for bitcoin height > 12..."
    local deadline=$((SECONDS + 120))
    while [ $SECONDS -lt $deadline ]; do
        local height
        height=$(btc_height)
        [ "${height}" -gt 12 ] && break
        sleep 1
    done
    if [ "$(btc_height)" -le 12 ]; then
        echo "FAIL: Bitcoin did not reach height 12 within 120s"
        exit 1
    fi
    echo "Bitcoin at height $(btc_height)"
}

restart_signet_slow() {
    echo "=== Restarting signet (BLOCKPRODUCTIONDELAY=30) ==="
    cd "${DOCKER_DIR}"
    docker compose -f compose-signet.yml down
    export BLOCKPRODUCTIONDELAY=30
    docker compose -f compose-signet.yml up -d

    echo "Waiting for bitcoind to come back up..."
    local deadline=$((SECONDS + 60))
    while [ $SECONDS -lt $deadline ]; do
        [ "$(btc_height)" -gt 0 ] && break
        sleep 1
    done
    if [ "$(btc_height)" -eq 0 ]; then
        echo "FAIL: Bitcoind did not come back up within 60s"
        exit 1
    fi

    GENESIS_HEIGHT=$(btc_height)
    echo "Genesis L1 height: ${GENESIS_HEIGHT}"
}

extract_datatool() {
    echo "=== Extracting datatool from ${DATATOOL_IMAGE} ==="
    docker create --name dt-extract "${DATATOOL_IMAGE}"
    docker cp dt-extract:/usr/local/bin/strata-datatool /tmp/strata-datatool
    docker rm dt-extract
    chmod +x /tmp/strata-datatool
}

generate_params() {
    echo "=== Generating params (all Sp1Groth16) ==="
    rm -rf "${DOCKER_DIR}/configs/generated"
    mkdir -p "${DOCKER_DIR}/configs/generated"

    cd "${DOCKER_DIR}"
    GENESIS_L1_HEIGHT="${GENESIS_HEIGHT}" \
        ./init-network.sh /tmp/strata-datatool

    # Verify no AlwaysAccept
    for f in configs/generated/{rollup-params,ol-params,asm-params}.json; do
        if grep -q "AlwaysAccept" "${f}"; then
            echo "FAIL: AlwaysAccept found in ${f}"
            exit 1
        fi
    done
    echo "All params use Sp1Groth16"

    # Source derived env vars (SEQUENCER_PUBKEY, SEQUENCER_PRIVATE_KEY, etc.)
    # shellcheck disable=SC1091
    set -a; . "${DOCKER_DIR}/.env.alpen"; set +a
    export GENESIS_L1_HEIGHT="${GENESIS_HEIGHT}"
}

start_sequencer_stack() {
    echo "=== Starting sequencer stack ==="
    cd "${REPO_ROOT}"
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" up -d

    wait_for_strata
    wait_for_alpen_client
}

assert_checkpoint_proof() {
    echo "=== Waiting for checkpoint proof (timeout ${CHECKPOINT_TIMEOUT}s) ==="
    local confirmed=0
    local deadline=$((SECONDS + CHECKPOINT_TIMEOUT))
    while [ $SECONDS -lt $deadline ]; do
        confirmed=$(get_confirmed_epoch)
        if [ "${confirmed}" -ge 1 ]; then
            echo "PASS: Checkpoint proof validated — confirmed epoch ${confirmed}"
            return 0
        fi
        sleep 5
    done
    echo "FAIL: Checkpoint proof not validated within ${CHECKPOINT_TIMEOUT}s"
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" logs strata 2>&1 \
        | grep -i "error\|fail\|invalid\|BN254" | tail -20
    exit 1
}

assert_ee_batch_proof() {
    echo "=== Waiting for EE batch proof (timeout ${BATCH_TIMEOUT}s) ==="
    local seq_no=0
    local deadline=$((SECONDS + BATCH_TIMEOUT))
    while [ $SECONDS -lt $deadline ]; do
        seq_no=$(get_snark_seq_no)
        if [ "${seq_no}" -gt 0 ]; then
            echo "PASS: EE batch proof accepted — seq_no ${seq_no}"
            return 0
        fi
        sleep 10
    done
    echo "FAIL: EE batch proof not accepted within ${BATCH_TIMEOUT}s"
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" logs alpen-client 2>&1 \
        | grep -i "error\|fail\|simulation\|chunk" | tail -20
    exit 1
}

assert_no_always_accept() {
    local vk
    vk=$(get_snark_update_vk)
    if [ "${vk}" = "AlwaysAccept" ]; then
        echo "FAIL: Runtime update_vk is AlwaysAccept"
        exit 1
    fi
    echo "PASS: Runtime update_vk is ${vk}"
}

# --- Orchestration ---

start_signet_fast
restart_signet_slow
extract_datatool
generate_params
start_sequencer_stack
assert_checkpoint_proof
assert_ee_batch_proof
assert_no_always_accept

echo ""
echo "=== ALL CHECKS PASSED ==="
