#!/bin/sh
set -eu
umask 027

CONFIG_PATH=${CONFIG_PATH:-/config/config.toml}
SEQUENCER_CONFIG_PATH=${SEQUENCER_CONFIG_PATH:-}
OL_PARAMS_PATH=${OL_PARAMS_PATH:-}
ASM_PARAMS_PATH=${ASM_PARAMS_PATH:-}
BITCOIND_RPC_URL=${BITCOIND_RPC_URL:-}
BITCOIND_RPC_USER=${BITCOIND_RPC_USER:-}
BITCOIND_RPC_PASSWORD=${BITCOIND_RPC_PASSWORD:-}

[ -f "${CONFIG_PATH}" ] || { echo "error: missing config '${CONFIG_PATH}'" >&2; exit 1; }
[ -f "${ASM_PARAMS_PATH}" ] || { echo "error: missing asm params '${ASM_PARAMS_PATH}'" >&2; exit 1; }
[ -f "${OL_PARAMS_PATH}" ] || { echo "error: missing ol params '${OL_PARAMS_PATH}'" >&2; exit 1; }

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

# Params are generated with their final genesis L1 anchor at init time (via
# datatool gen-l1-anchor, pinned to GENESIS_L1_HEIGHT). The node consumes them
# as-is — there is no runtime genesis patching.
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
  ${SEQUENCER_ARGS} \
  ${EXTRA_ARGS} \
  ${CONFIG_OVERRIDES} \
  "$@"
