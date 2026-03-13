#!/bin/sh
set -eu
umask 027

CONFIG_PATH=${CONFIG_PATH:-/config/config.toml}
SEQUENCER_CONFIG_PATH=${SEQUENCER_CONFIG_PATH:-}
PARAM_PATH=${PARAM_PATH:-/config/params.json}
OL_PARAMS_PATH=${OL_PARAMS_PATH:-}
ASM_PARAMS_PATH=${ASM_PARAMS_PATH:-}
BITCOIND_RPC_URL=${BITCOIND_RPC_URL:-}
BITCOIND_RPC_USER=${BITCOIND_RPC_USER:-}
BITCOIND_RPC_PASSWORD=${BITCOIND_RPC_PASSWORD:-}

[ -f "${CONFIG_PATH}" ] || { echo "error: missing config '${CONFIG_PATH}'" >&2; exit 1; }
[ -f "${PARAM_PATH}" ] || { echo "error: missing params '${PARAM_PATH}'" >&2; exit 1; }

default_sequencer_config_path() {
    config_path="$1"
    dir_path=$(dirname "${config_path}")
    printf "%s/sequencer.toml\n" "${dir_path}"
}

sequencer_config_path() {
    config_path="$1"
    if [ -n "${SEQUENCER_CONFIG_PATH}" ]; then
        printf "%s\n" "${SEQUENCER_CONFIG_PATH}"
    else
        default_sequencer_config_path "${config_path}"
    fi
}

requires_sequencer_config() {
    if grep -Eq '^[[:space:]]*is_sequencer[[:space:]]*=[[:space:]]*true' "${CONFIG_PATH}"; then
        return 0
    fi

    for arg in "$@"; do
        if [ "${arg}" = "--sequencer" ]; then
            return 0
        fi
    done

    return 1
}

# Runtime genesis patching.
#
# If params were generated with a real GenesisL1View (via datatool genl1view
# at init time), the genesis height will already be > 0 and all fields
# (next_target, epoch_start_timestamp, last_11_timestamps) will be correct.
# In that case we skip patching.
#
# If params have genesis height == 0 (placeholder from init without RPC),
# we patch height + blkid from the current L1 tip.  This is a partial patch
# (next_target and timestamps are NOT updated) which is acceptable on regtest
# where difficulty is constant, but NOT sufficient for signet/mainnet.  For
# non-regtest networks, generate params with BITCOIN_RPC_* at init time.
CURRENT_GENESIS_HEIGHT=$(jq -r '.genesis_l1_view.blk.height // 0' "${PARAM_PATH}" 2>/dev/null || echo "0")

if [ -n "${BITCOIND_RPC_URL}" ] && [ "${CURRENT_GENESIS_HEIGHT}" -eq 0 ]; then
    rpc_call() {
        curl -sf -u "${BITCOIND_RPC_USER}:${BITCOIND_RPC_PASSWORD}" \
            -d "{\"jsonrpc\":\"1.0\",\"method\":\"$1\",\"params\":$2}" \
            "${BITCOIND_RPC_URL}"
    }

    echo "genesis height is 0 (placeholder) — patching from L1 tip..."
    INFO=$(rpc_call getblockchaininfo '[]')
    TIP_HEIGHT=$(echo "${INFO}" | jq -r '.result.blocks')
    TIP_HASH=$(echo "${INFO}" | jq -r '.result.bestblockhash')

    if [ -z "${TIP_HEIGHT}" ] || [ "${TIP_HEIGHT}" = "null" ]; then
        echo "error: failed to get L1 tip from ${BITCOIND_RPC_URL}" >&2
        exit 1
    fi

    echo "L1 tip: height=${TIP_HEIGHT} hash=${TIP_HASH}"

    # Patch rollup-params.json: update genesis_l1_view.blk
    PATCHED_PARAMS="/app/data/rollup-params.json"
    jq --argjson h "${TIP_HEIGHT}" --arg id "${TIP_HASH}" \
        '.genesis_l1_view.blk.height = $h | .genesis_l1_view.blk.blkid = $id' \
        "${PARAM_PATH}" > "${PATCHED_PARAMS}"
    PARAM_PATH="${PATCHED_PARAMS}"

    # Patch ol-params.json if provided
    if [ -n "${OL_PARAMS_PATH}" ] && [ -f "${OL_PARAMS_PATH}" ]; then
        PATCHED_OL="/app/data/ol-params.json"
        jq --argjson h "${TIP_HEIGHT}" --arg id "${TIP_HASH}" \
            '.last_l1_block.height = $h | .last_l1_block.blkid = $id' \
            "${OL_PARAMS_PATH}" > "${PATCHED_OL}"
        OL_PARAMS_PATH="${PATCHED_OL}"
    fi

    # Patch asm-params.json if provided
    if [ -n "${ASM_PARAMS_PATH}" ] && [ -f "${ASM_PARAMS_PATH}" ]; then
        PATCHED_ASM="/app/data/asm-params.json"
        jq --argjson h "${TIP_HEIGHT}" --arg id "${TIP_HASH}" \
            '.l1_view.blk.height = $h | .l1_view.blk.blkid = $id' \
            "${ASM_PARAMS_PATH}" > "${PATCHED_ASM}"
        ASM_PARAMS_PATH="${PATCHED_ASM}"
    fi
elif [ "${CURRENT_GENESIS_HEIGHT}" -gt 0 ]; then
    echo "genesis height is ${CURRENT_GENESIS_HEIGHT} — params already initialized, skipping patching"
fi

EXTRA_ARGS=""
if [ -n "${OL_PARAMS_PATH}" ] && [ -f "${OL_PARAMS_PATH}" ]; then
    EXTRA_ARGS="${EXTRA_ARGS} --ol-params ${OL_PARAMS_PATH}"
fi
if [ -n "${ASM_PARAMS_PATH}" ] && [ -f "${ASM_PARAMS_PATH}" ]; then
    EXTRA_ARGS="${EXTRA_ARGS} --asm-params ${ASM_PARAMS_PATH}"
fi

# Override config values from environment variables so a single config TOML
# works for both regtest and signet.
CONFIG_OVERRIDES=""
if [ -n "${BITCOIND_RPC_URL}" ]; then
    CONFIG_OVERRIDES="${CONFIG_OVERRIDES} -o bitcoind.rpc_url=${BITCOIND_RPC_URL}"
fi
if [ -n "${BITCOIND_RPC_USER}" ]; then
    CONFIG_OVERRIDES="${CONFIG_OVERRIDES} -o bitcoind.rpc_user=${BITCOIND_RPC_USER}"
fi
if [ -n "${BITCOIND_RPC_PASSWORD}" ]; then
    CONFIG_OVERRIDES="${CONFIG_OVERRIDES} -o bitcoind.rpc_password=${BITCOIND_RPC_PASSWORD}"
fi

BITCOIN_NETWORK="${BITCOIN_NETWORK:-regtest}"
CONFIG_OVERRIDES="${CONFIG_OVERRIDES} -o bitcoind.network=${BITCOIN_NETWORK}"

SEQUENCER_ARGS=""
if requires_sequencer_config "$@"; then
    RESOLVED_SEQUENCER_CONFIG_PATH=$(sequencer_config_path "${CONFIG_PATH}")
    [ -f "${RESOLVED_SEQUENCER_CONFIG_PATH}" ] || {
        echo "error: missing sequencer config '${RESOLVED_SEQUENCER_CONFIG_PATH}'" >&2
        exit 1
    }

    # Patch OL block time from env var so infra can override without re-running init.
    OL_BLOCK_TIME_MS="${OL_BLOCK_TIME_MS:-}"
    if [ -n "${OL_BLOCK_TIME_MS}" ]; then
        PATCHED_SEQ_CONFIG="/app/data/sequencer.toml"
        sed "s/^ol_block_time_ms.*/ol_block_time_ms = ${OL_BLOCK_TIME_MS}/" \
            "${RESOLVED_SEQUENCER_CONFIG_PATH}" > "${PATCHED_SEQ_CONFIG}"
        RESOLVED_SEQUENCER_CONFIG_PATH="${PATCHED_SEQ_CONFIG}"
        echo "patched ol_block_time_ms=${OL_BLOCK_TIME_MS}"
    fi

    SEQUENCER_ARGS="--sequencer-config ${RESOLVED_SEQUENCER_CONFIG_PATH}"
fi

# Intentional word splitting of multi-arg strings
# shellcheck disable=SC2086
exec strata \
  --config "${CONFIG_PATH}" \
  --rollup-params "${PARAM_PATH}" \
  ${SEQUENCER_ARGS} \
  ${EXTRA_ARGS} \
  ${CONFIG_OVERRIDES} \
  "$@"
