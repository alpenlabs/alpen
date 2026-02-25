#!/bin/sh
set -eu
umask 027

CONFIG_PATH=${CONFIG_PATH:-/config/config.toml}
PARAM_PATH=${PARAM_PATH:-/config/params.json}
OL_PARAMS_PATH=${OL_PARAMS_PATH:-}
BITCOIND_RPC_URL=${BITCOIND_RPC_URL:-}
BITCOIND_RPC_USER=${BITCOIND_RPC_USER:-}
BITCOIND_RPC_PASSWORD=${BITCOIND_RPC_PASSWORD:-}

[ -f "${CONFIG_PATH}" ] || { echo "error: missing config '${CONFIG_PATH}'" >&2; exit 1; }
[ -f "${PARAM_PATH}" ] || { echo "error: missing params '${PARAM_PATH}'" >&2; exit 1; }

# If BITCOIND_RPC_URL is set, query the L1 tip and patch genesis params.
# This is needed because the L1 reader doesn't store block 0 in the canonical
# chain, so genesis_l1_height must be > 0.
if [ -n "${BITCOIND_RPC_URL}" ]; then
    rpc_call() {
        curl -sf -u "${BITCOIND_RPC_USER}:${BITCOIND_RPC_PASSWORD}" \
            -d "{\"jsonrpc\":\"1.0\",\"method\":\"$1\",\"params\":$2}" \
            "${BITCOIND_RPC_URL}"
    }

    echo "querying bitcoind for L1 tip..."
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
fi

EXTRA_ARGS=""
if [ -n "${OL_PARAMS_PATH}" ] && [ -f "${OL_PARAMS_PATH}" ]; then
    EXTRA_ARGS="--ol-params ${OL_PARAMS_PATH}"
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

exec strata \
  --config "${CONFIG_PATH}" \
  --rollup-params "${PARAM_PATH}" \
  ${EXTRA_ARGS} \
  ${CONFIG_OVERRIDES} \
  "$@"
