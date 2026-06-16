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
PROOF_TIMEOUT="${PROOF_TIMEOUT:-2400}"
RPC_READY_TIMEOUT="${RPC_READY_TIMEOUT:-120}"
FAILURE_REASON_FILE="${SCRIPT_DIR}/e2e-failure-reason.txt"
WARN_ERROR_SUMMARY_FILE="${SCRIPT_DIR}/e2e-warn-error-summary.txt"

DATATOOL_IMAGE="${ECR_REGISTRY}/strata-datatool:${IMAGE_TAG}"

ALPEN_ACCOUNT_ID="0101010101010101010101010101010101010101010101010101010101010101"
CHAIN_STATUS_PAYLOAD='{"jsonrpc":"2.0","method":"strata_getChainStatus","params":[],"id":1}'
SNARK_STATE_PAYLOAD='{"jsonrpc":"2.0","method":"strata_getSnarkAccountStateByTag","params":["'"${ALPEN_ACCOUNT_ID}"'","latest"],"id":1}'
EE_BLOCK_NUMBER_PAYLOAD='{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
STACK_LOG_SINCE=""
LAST_FAILURE_REASON=""

# Signet config
export SIGNET_IMAGE="public.ecr.aws/alpenlabs/signet:tmpconf2"
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
export CHAIN_SPEC=testnet
# P2TR BOSD for the bridge safe harbour address (required by init-network.sh).
# CI-only throwaway value — 04 (P2TR type tag) + 32-byte x-only pubkey derived
# from the "abandon" mnemonic. Not used for real funds.
export SAFE_HARBOUR_ADDRESS="0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"

# --- Low-level helpers ---

record_failure() {
    local reason="$1"

    if [ ! -s "${FAILURE_REASON_FILE}" ]; then
        printf '%s\n' "${reason}" | strip_ansi > "${FAILURE_REASON_FILE}"
    fi
}

strip_ansi() {
    sed -E $'s/\x1B\\[[0-9;?]*[ -/]*[@-~]//g'
}

fail() {
    local reason="$1"

    printf 'FAIL: %s\n' "${reason}" | strip_ansi
    record_failure "${reason}"
    exit 1
}

on_error() {
    local line="$1"
    local command="$2"

    record_failure "command failed at line ${line}: ${command}"
}

cleanup() {
    [ -n "${LOGS_PID:-}" ] && kill "${LOGS_PID}" 2>/dev/null || true

    echo "=== Collecting final state ==="
    {
        echo "--- Chain Status ---"
        ol_rpc "${CHAIN_STATUS_PAYLOAD}" 2>/dev/null | jq . 2>/dev/null || echo "(unavailable)"
        echo ""
        echo "--- Snark Account State ---"
        ol_rpc "${SNARK_STATE_PAYLOAD}" 2>/dev/null | jq . 2>/dev/null || echo "(unavailable)"
        echo ""
        echo "--- Bitcoin Height ---"
        btc_height 2>/dev/null || echo "(unavailable)"
        echo ""
        echo "--- EE Latest Block ---"
        ee_rpc "${EE_BLOCK_NUMBER_PAYLOAD}" 2>/dev/null | jq . 2>/dev/null || echo "(unavailable)"
    } > "${SCRIPT_DIR}/e2e-state.txt" 2>&1

    {
        echo "--- OL Params ---"
        cat "${DOCKER_DIR}/configs/generated/ol-params.json" 2>/dev/null | jq . 2>/dev/null || echo "(unavailable)"
        echo ""
        echo "--- ASM Params ---"
        cat "${DOCKER_DIR}/configs/generated/asm-params.json" 2>/dev/null | jq . 2>/dev/null || echo "(unavailable)"
    } > "${SCRIPT_DIR}/e2e-params.txt" 2>&1

    {
        echo "--- .env.alpen ---"
        cat "${DOCKER_DIR}/configs/generated/.env.alpen" 2>/dev/null || echo "(unavailable)"
    } > "${SCRIPT_DIR}/e2e-env.txt" 2>&1

    echo "=== Collecting logs ==="
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" logs --no-color > "${SCRIPT_DIR}/e2e-logs.txt" 2>&1 || true
    docker compose -f "${DOCKER_DIR}/compose-signet.yml" logs --no-color >> "${SCRIPT_DIR}/e2e-logs.txt" 2>&1 || true
    python3 "${SCRIPT_DIR}/summarize-warn-error-logs.py" "${SCRIPT_DIR}/e2e-logs.txt" > "${WARN_ERROR_SUMMARY_FILE}" || true
    echo "=== Tearing down ==="
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" down -v 2>/dev/null || true
    docker compose -f "${DOCKER_DIR}/compose-signet.yml" down -v 2>/dev/null || true
}
trap 'on_error "${LINENO}" "${BASH_COMMAND}"' ERR
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
ee_rpc() { json_rpc "$1" 8545; }

assert_rpc_result() {
    local label="$1"
    local payload="$2"
    local port="$3"
    local response

    if ! response=$(json_rpc "${payload}" "${port}"); then
        LAST_FAILURE_REASON="${label} RPC request failed"
        echo "FAIL: ${LAST_FAILURE_REASON}"
        return 1
    fi

    local validation_error
    validation_error=$(printf '%s' "${response}" | python3 "${SCRIPT_DIR}/validate-json-rpc.py" "${label}") || {
        LAST_FAILURE_REASON="${validation_error}"
        echo "FAIL: ${LAST_FAILURE_REASON}"
        return 1
    }
}

wait_for_service() {
    local label="$1"
    local port="$2"
    local method="$3"
    local timeout="${4:-120}"

    echo "Waiting for ${label}..."
    local deadline=$((SECONDS + timeout))
    while [ $SECONDS -lt $deadline ]; do
        local payload='{"jsonrpc":"2.0","method":"'"${method}"'","params":[],"id":1}'
        if assert_rpc_result "${label}.${method}" "${payload}" "${port}" >/dev/null 2>&1; then
            echo "${label} ready"
            return 0
        fi
        sleep 2
    done
    fail "${label} not reachable within ${timeout}s"
}

wait_for_strata()       { wait_for_service "strata"       8432 "strata_protocolVersion"; }
wait_for_alpen_client() { wait_for_service "alpen-client" 8545 "eth_blockNumber"; }

wait_for_rpc_result() {
    local label="$1"
    local payload="$2"
    local port="$3"
    local timeout="${4:-${RPC_READY_TIMEOUT}}"

    echo "Waiting for ${label} RPC..."
    local deadline=$((SECONDS + timeout))
    while [ $SECONDS -lt $deadline ]; do
        if assert_rpc_result "${label}" "${payload}" "${port}" >/dev/null 2>&1; then
            echo "${label} RPC ready"
            return 0
        fi
        sleep 2
    done

    fail "${LAST_FAILURE_REASON:-${label} RPC not ready within ${timeout}s}"
}

assert_required_rpc_methods() {
    echo "=== Validating required RPC methods ==="
    wait_for_rpc_result "strata_getChainStatus" "${CHAIN_STATUS_PAYLOAD}" 8432
    wait_for_rpc_result "strata_getSnarkAccountState" "${SNARK_STATE_PAYLOAD}" 8432
    wait_for_rpc_result "eth_blockNumber" "${EE_BLOCK_NUMBER_PAYLOAD}" 8545
}

compose_stack() {
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" "$@"
}

stack_service_states() {
    local service
    local cid
    local inspect

    for service in strata strata-signer alpen-client; do
        cid=$(compose_stack ps -q "${service}")
        if [ -z "${cid}" ]; then
            printf '%s\tmissing\t\t\n' "${service}"
            continue
        fi

        inspect=$(docker inspect \
            --format '{{.State.Status}} {{.State.ExitCode}} {{.RestartCount}}' \
            "${cid}")
        printf '%s\t%s\n' "${service}" "${inspect}"
    done
}

stack_state_failure_reason() {
    local service
    local status
    local exit_code
    local restarts

    while IFS=$'\t ' read -r service status exit_code restarts; do
        if [ "${status}" = "missing" ]; then
            echo "${service} container is missing"
            return 0
        fi
        if [ "${status}" != "running" ] || [ "${exit_code}" != "0" ] || [ "${restarts}" != "0" ]; then
            echo "${service} unhealthy: status=${status}, exit_code=${exit_code}, restarts=${restarts}"
            return 0
        fi
    done
}

stack_recent_logs() {
    if [ -n "${STACK_LOG_SINCE}" ]; then
        compose_stack logs --no-color --since "${STACK_LOG_SINCE}" strata alpen-client 2>/dev/null || true
    fi
}

fatal_log_failure_reason() {
    local fatal_logs

    fatal_logs=$(grep -E "critical task exited with error|chunked envelope watcher exited with error|smart fee estimate unavailable|Method not found" || true)
    if [ -n "${fatal_logs}" ]; then
        echo "fatal service error detected: $(printf '%s\n' "${fatal_logs}" | tail -1)"
    fi
}

stack_failure_reason() {
    local state_reason
    local log_reason

    state_reason=$(stack_service_states | stack_state_failure_reason)
    if [ -n "${state_reason}" ]; then
        echo "${state_reason}"
        return 0
    fi

    log_reason=$(stack_recent_logs | fatal_log_failure_reason)
    if [ -n "${log_reason}" ]; then
        echo "${log_reason}"
    fi
}

assert_stack_healthy_or_exit() {
    local reason

    reason=$(stack_failure_reason)
    if [ -n "${reason}" ]; then
        LAST_FAILURE_REASON="${reason}"
        echo "FAIL: ${LAST_FAILURE_REASON}"
        compose_stack logs --tail=120 strata alpen-client || true
        fail "${LAST_FAILURE_REASON}"
    fi
}

compose_signet_up() {
    local label="$1"
    local deadline=$((SECONDS + 300))
    local attempt=1
    local output

    while [ $SECONDS -lt $deadline ]; do
        if output=$(docker compose -f compose-signet.yml up -d 2>&1); then
            printf '%s\n' "${output}" | tail -1
            return 0
        fi

        printf '%s\n' "${output}" | tail -20
        echo "Signet ${label} start failed on attempt ${attempt}; retrying in 30s..."
        attempt=$((attempt + 1))
        sleep 30
    done

    echo "FAIL: Signet ${label} did not start within 300s"
    return 1
}

btc_height() {
    docker exec docker-bitcoind-1 bitcoin-cli -signet getblockcount 2>/dev/null || echo 0
}

parse_result() {
    jq -r ".result$1" 2>/dev/null
}

get_confirmed_epoch() {
    ol_rpc "${CHAIN_STATUS_PAYLOAD}" | parse_result ".confirmed.epoch // 0" || echo 0
}

get_tip_epoch() {
    ol_rpc "${CHAIN_STATUS_PAYLOAD}" | parse_result ".tip.epoch // 0" || echo 0
}

get_snark_seq_no() {
    ol_rpc "${SNARK_STATE_PAYLOAD}" | parse_result ".seq_no // 0" || echo 0
}

get_snark_update_vk() {
    ol_rpc "${SNARK_STATE_PAYLOAD}" | parse_result '.update_vk // "unknown"' || echo "unknown"
}

# --- Step functions ---

preflight_cleanup() {
    echo "=== Pre-flight cleanup: removing stale containers and volumes ==="
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" down -v 2>/dev/null || true
    docker compose -f "${DOCKER_DIR}/compose-signet.yml" down -v 2>/dev/null || true
}

start_signet_fast() {
    echo "=== Starting signet (BLOCKPRODUCTIONDELAY=0) ==="
    cd "${DOCKER_DIR}"
    export BLOCKPRODUCTIONDELAY=0
    compose_signet_up "fast"

    echo "Waiting for bitcoin height > 101 (coinbase maturity)..."
    local deadline=$((SECONDS + 300))
    local last_print=0
    while [ $SECONDS -lt $deadline ]; do
        local height
        height=$(btc_height)
        [ "${height}" -gt 101 ] && break
        if [ "${height}" -ge $((last_print + 10)) ]; then
            echo "  bitcoin height: ${height}/101"
            last_print="${height}"
        fi
        sleep 1
    done
    if [ "$(btc_height)" -le 101 ]; then
        fail "Bitcoin did not reach height 101 within 300s"
    fi
    echo "Bitcoin at height $(btc_height)"
}

restart_signet_slow() {
    echo "=== Restarting signet (BLOCKPRODUCTIONDELAY=30) ==="
    cd "${DOCKER_DIR}"
    docker compose -f compose-signet.yml down
    export BLOCKPRODUCTIONDELAY=30
    compose_signet_up "slow"

    echo "Waiting for bitcoind to come back up..."
    local deadline=$((SECONDS + 60))
    while [ $SECONDS -lt $deadline ]; do
        [ "$(btc_height)" -gt 0 ] && break
        sleep 1
    done
    if [ "$(btc_height)" -eq 0 ]; then
        fail "Bitcoind did not come back up within 60s"
    fi

    GENESIS_HEIGHT=$(btc_height)
    echo "Genesis L1 height: ${GENESIS_HEIGHT}"
}

extract_datatool() {
    echo "=== Extracting datatool from ${DATATOOL_IMAGE} ==="
    docker create --name dt-extract "${DATATOOL_IMAGE}" >/dev/null
    docker cp dt-extract:/usr/local/bin/strata-datatool /tmp/strata-datatool
    docker rm dt-extract >/dev/null
    chmod +x /tmp/strata-datatool

    # Copy ELFs from prebuilt images into docker/elfs/ so the base compose
    # volume mount has real ELFs instead of an empty directory.
    echo "=== Extracting ELFs from prebuilt images ==="
    mkdir -p "${DOCKER_DIR}/elfs"

    docker create --name elf-strata "${ECR_REGISTRY}/strata:${IMAGE_TAG}" >/dev/null
    docker cp elf-strata:/app/elfs/sp1/. "${DOCKER_DIR}/elfs/"
    docker rm elf-strata >/dev/null

    docker create --name elf-alpen "${ECR_REGISTRY}/alpen-client:${IMAGE_TAG}" >/dev/null
    docker cp elf-alpen:/app/elfs/sp1/. "${DOCKER_DIR}/elfs/"
    docker rm elf-alpen >/dev/null

    echo "ELFs extracted:"
    ls "${DOCKER_DIR}/elfs/"
}

generate_params() {
    echo "=== Generating params (all Sp1Groth16) ==="
    rm -rf "${DOCKER_DIR}/configs/generated"
    mkdir -p "${DOCKER_DIR}/configs/generated"

    # Base compose expects docker/.env — create it from exported env vars
    # so compose variable substitution works.
    touch "${DOCKER_DIR}/.env"

    cd "${DOCKER_DIR}"
    export ENV_FILE="${DOCKER_DIR}/configs/generated/.env.alpen"
    GENESIS_L1_HEIGHT="${GENESIS_HEIGHT}" \
        ./init-network.sh /tmp/strata-datatool

    # Verify no AlwaysAccept
    for f in configs/generated/{ol-params,asm-params}.json; do
        if grep -q "AlwaysAccept" "${f}"; then
            fail "AlwaysAccept found in ${f}"
        fi
    done
    echo "All params use Sp1Groth16"

    for f in configs/generated/{ol-params,asm-params}.json; do
        echo "::group::${f}"
        cat "${f}"
        echo "::endgroup::"
    done

    # Source derived env vars (SEQUENCER_PUBKEY, SEQUENCER_PRIVATE_KEY, etc.)
    set -a
    # shellcheck disable=SC1091
    . "${DOCKER_DIR}/configs/generated/.env.alpen"
    set +a
    export GENESIS_L1_HEIGHT="${GENESIS_HEIGHT}"
}

start_sequencer_stack() {
    echo "=== Starting sequencer stack ==="
    cd "${REPO_ROOT}"
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" up -d 2>&1 | tail -3
    STACK_LOG_SINCE=$(date -u +%Y-%m-%dT%H:%M:%SZ)

    wait_for_strata
    wait_for_alpen_client
    assert_required_rpc_methods
    assert_stack_healthy_or_exit

    # Stream container logs to stdout so they appear in GH Actions
    docker compose -f "${DOCKER_DIR}/compose-ol-el-seq.yml" -f "${SCRIPT_DIR}/compose-override.yml" logs -f &
    LOGS_PID=$!
}

assert_proofs() {
    local timeout="${PROOF_TIMEOUT}"
    echo "=== Waiting for proofs (timeout ${timeout}s) ==="

    local sau_epoch=-1
    local sau_accepted=false
    local sau_confirmed=false
    local deadline=$((SECONDS + timeout))

    while [ $SECONDS -lt $deadline ]; do
        assert_stack_healthy_or_exit

        if [ "${sau_accepted}" = false ]; then
            local seq_no
            seq_no=$(get_snark_seq_no)
            if [ "${seq_no}" -gt 0 ]; then
                sau_epoch=$(get_tip_epoch)
                echo "PASS: SAU accepted by OL — seq_no ${seq_no}, epoch ${sau_epoch}"
                sau_accepted=true
            fi
        fi

        if [ "${sau_accepted}" = true ] && [ "${sau_confirmed}" = false ]; then
            local confirmed
            confirmed=$(get_confirmed_epoch)
            if [ "${confirmed}" -ge "${sau_epoch}" ]; then
                echo "PASS: SAU with seq_no ${seq_no}, received in epoch ${sau_epoch}, confirmed — confirmed epoch ${confirmed}"
                sau_confirmed=true
            fi
        fi

        if [ "${sau_accepted}" = true ] && [ "${sau_confirmed}" = true ]; then
            return 0
        fi

        sleep 5
    done

    if [ "${sau_accepted}" = false ]; then
        fail "SAU not accepted within ${timeout}s"
    fi
    if [ "${sau_confirmed}" = false ]; then
        fail "SAU epoch ${sau_epoch} not confirmed within ${timeout}s (confirmed=$(get_confirmed_epoch))"
    fi
}

assert_no_always_accept() {
    local vk
    vk=$(get_snark_update_vk)
    if [ "${vk}" = "AlwaysAccept" ]; then
        fail "Runtime update_vk is AlwaysAccept"
    fi
    echo "PASS: Runtime update_vk is ${vk}"
}

# --- Orchestration ---

rm -f "${FAILURE_REASON_FILE}"

preflight_cleanup
start_signet_fast
restart_signet_slow
extract_datatool
generate_params
start_sequencer_stack
assert_proofs
assert_no_always_accept

echo ""
echo "=== ALL CHECKS PASSED ==="
