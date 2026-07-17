#!/bin/sh

# Fail fast on errors and unset variables
set -eu

# Restrict default permissions for newly created files
umask 027

if [ "${1-}" = "help" ] || [ "${1-}" = "--help" ] || [ "${1-}" = "-h" ]; then
    exec alpen-client --help
fi

# Build command from environment variables.
# Set SEQUENCER_MODE=true to run as sequencer (default: fullnode).

SEQUENCER_MODE="${SEQUENCER_MODE:-false}"
SEQUENCER_PUBKEY="${SEQUENCER_PUBKEY:?SEQUENCER_PUBKEY must be set}"
ALPEN_PARAMS_PATH="${ALPEN_PARAMS_PATH:-/app/configs/generated/alpen-params.json}"

if [ "${DUMMY_OL_CLIENT:-0}" = "1" ]; then
    set -- --dummy-ol-client "$@"
else
    set -- \
        --ol-client-url "${OL_CLIENT_URL:-ws://strata:8432}" \
        "$@"
fi

# Sequencer-only: submit URL, DA flags, and BTC credentials
if [ "${SEQUENCER_MODE}" = "true" ]; then
    BITCOIND_RPC_URL="${BITCOIND_RPC_URL:?BITCOIND_RPC_URL must be set}"
    BITCOIND_RPC_USER="${BITCOIND_RPC_USER:?BITCOIND_RPC_USER must be set}"
    BITCOIND_RPC_PASSWORD="${BITCOIND_RPC_PASSWORD:?BITCOIND_RPC_PASSWORD must be set}"
    STRATA_SUBMIT_RPC_TOKEN="${STRATA_SUBMIT_RPC_TOKEN:?STRATA_SUBMIT_RPC_TOKEN must be set}"
    BTCIO_FEE_POLICY="${BTCIO_FEE_POLICY:-bitcoind}"

    set -- \
        --sequencer \
        --ol-submit-url "${OL_SUBMIT_URL:-ws://strata:8435}" \
        --btc-rpc-url "${BITCOIND_RPC_URL}" \
        --btc-rpc-user "${BITCOIND_RPC_USER}" \
        --btc-rpc-password "${BITCOIND_RPC_PASSWORD}" \
        --btcio-fee-policy "${BTCIO_FEE_POLICY}" \
        "$@"

    if [ -n "${BTCIO_CONF_TARGET:-}" ]; then
        set -- "$@" --btcio-conf-target "${BTCIO_CONF_TARGET}"
    fi

    if [ -n "${BTCIO_FEE_RATE:-}" ]; then
        set -- "$@" --btcio-fee-rate "${BTCIO_FEE_RATE}"
    fi

    if [ -n "${BTCIO_MEMPOOL_BASE_URL:-}" ]; then
        set -- "$@" --btcio-mempool-base-url "${BTCIO_MEMPOOL_BASE_URL}"
    fi

    if [ -n "${BTCIO_MEMPOOL_TIER:-}" ]; then
        set -- "$@" --btcio-mempool-tier "${BTCIO_MEMPOOL_TIER}"
    fi
fi

exec alpen-client \
    --sequencer-pubkey "${SEQUENCER_PUBKEY}" \
    --alpen-params "${ALPEN_PARAMS_PATH}" \
    --datadir "${DATADIR:-/app/data}" \
    --addr 0.0.0.0 \
    --http \
    --http.addr 0.0.0.0 \
    --http.port "${HTTP_PORT:-8545}" \
    --http.api "${HTTP_API:-eth,net,web3,txpool,admin,debug}" \
    --ws \
    --ws.addr 0.0.0.0 \
    --ws.port "${WS_PORT:-8546}" \
    --ws.api "${WS_API:-eth,net,web3,txpool}" \
    --authrpc.addr 0.0.0.0 \
    --authrpc.port "${AUTHRPC_PORT:-8551}" \
    --authrpc.jwtsecret "${JWT_SECRET:-/app/keys/jwt.hex}" \
    --l1-reorg-safe-depth "${L1_REORG_SAFE_DEPTH:-4}" \
    --batch-sealing-block-count "${BATCH_SEALING_BLOCK_COUNT:-120}" \
    --txpool.minimal-protocol-fee "${TXPOOL_MIN_PROTOCOL_FEE:-0}" \
    --genesis-l1-height "${GENESIS_L1_HEIGHT:?GENESIS_L1_HEIGHT must be set}" \
    "$@"
