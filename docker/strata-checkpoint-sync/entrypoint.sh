#!/bin/sh
set -eu
umask 027

CONFIG_PATH=${CONFIG_PATH:-/config/config.toml}
OL_PARAMS_PATH=${OL_PARAMS_PATH:-}
ASM_PARAMS_PATH=${ASM_PARAMS_PATH:-}
BITCOIND_RPC_URL=${BITCOIND_RPC_URL:-}
BITCOIND_RPC_USER=${BITCOIND_RPC_USER:-}
BITCOIND_RPC_PASSWORD=${BITCOIND_RPC_PASSWORD:-}

[ -f "${CONFIG_PATH}" ] || { echo "error: missing config '${CONFIG_PATH}'" >&2; exit 1; }
[ -f "${ASM_PARAMS_PATH}" ] || { echo "error: missing asm params '${ASM_PARAMS_PATH}'" >&2; exit 1; }
[ -f "${OL_PARAMS_PATH}" ] || { echo "error: missing ol params '${OL_PARAMS_PATH}'" >&2; exit 1; }

# This binary is built without the `sequencer` feature, so it has no block
# assembly, writer, or checkpoint-proving code. A config with is_sequencer=true
# would start a broken half-sequencer (mempool + fork-choice, no block
# production). Reject it instead of silently misbehaving.
if grep -Eq '^[[:space:]]*is_sequencer[[:space:]]*=[[:space:]]*true' "${CONFIG_PATH}"; then
    echo "error: is_sequencer=true in '${CONFIG_PATH}', but this is a checkpoint-sync image (built without the sequencer feature)" >&2
    exit 1
fi

for arg in "$@"; do
    if [ "${arg}" = "--sequencer" ]; then
        echo "error: --sequencer passed, but this is a checkpoint-sync image (built without the sequencer feature)" >&2
        exit 1
    fi
done

# Params must be loaded correct and complete. The OL genesis block id is a
# function of the L1 genesis height/blkid and the rest of the params, so it
# cannot be patched piecemeal at runtime; generate params with the right
# GenesisL1View ahead of time (datatool genl1view) and mount them here.

EXTRA_ARGS=""
EXTRA_ARGS="${EXTRA_ARGS} --ol-params ${OL_PARAMS_PATH}"
EXTRA_ARGS="${EXTRA_ARGS} --asm-params ${ASM_PARAMS_PATH}"

# Override config values from environment variables so a single config TOML can
# point at a local or remote signet fullnode.
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

BITCOIN_NETWORK="${BITCOIN_NETWORK:-signet}"
CONFIG_OVERRIDES="${CONFIG_OVERRIDES} -o bitcoind.network=${BITCOIN_NETWORK}"

# Intentional word splitting of multi-arg strings
# shellcheck disable=SC2086
exec strata \
  --config "${CONFIG_PATH}" \
  ${EXTRA_ARGS} \
  ${CONFIG_OVERRIDES} \
  "$@"
