#!/bin/sh

# Fail fast on errors and unset variables
set -eu

# Restrict default permissions for newly created files
umask 027

if [ "${1-}" = "help" ] || [ "${1-}" = "--help" ] || [ "${1-}" = "-h" ]; then
    exec alpen-client --help
fi

# Build command from environment variables.

SEQUENCER_PUBKEY="${SEQUENCER_PUBKEY:?SEQUENCER_PUBKEY must be set}"
CHAIN_SPEC="${CHAIN_SPEC:-dev}"
EE_DA_MAGIC_BYTES="${EE_DA_MAGIC_BYTES:-ALPN}"
BITCOIND_RPC_URL="${BITCOIND_RPC_URL:?BITCOIND_RPC_URL must be set}"
BITCOIND_RPC_USER="${BITCOIND_RPC_USER:?BITCOIND_RPC_USER must be set}"
BITCOIND_RPC_PASSWORD="${BITCOIND_RPC_PASSWORD:?BITCOIND_RPC_PASSWORD must be set}"

if [ "${DUMMY_OL_CLIENT:-0}" = "1" ]; then
    set -- --dummy-ol-client "$@"
else
    STRATA_SUBMIT_RPC_TOKEN="${STRATA_SUBMIT_RPC_TOKEN:?STRATA_SUBMIT_RPC_TOKEN must be set}"
    set -- \
        --ol-client-url "${OL_CLIENT_URL:-ws://strata:8432}" \
        --ol-submit-url "${OL_SUBMIT_URL:-ws://strata:8435}" \
        "$@"
fi

exec alpen-client \
    --sequencer \
    --sequencer-pubkey "${SEQUENCER_PUBKEY}" \
    --custom-chain "${CHAIN_SPEC}" \
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
    --ee-da-magic-bytes "${EE_DA_MAGIC_BYTES}" \
    --btc-rpc-url "${BITCOIND_RPC_URL}" \
    --btc-rpc-user "${BITCOIND_RPC_USER}" \
    --btc-rpc-password "${BITCOIND_RPC_PASSWORD}" \
    --l1-reorg-safe-depth "${L1_REORG_SAFE_DEPTH:-4}" \
    --batch-sealing-block-count "${BATCH_SEALING_BLOCK_COUNT:-120}" \
    --bridge-denomination "${BRIDGE_DENOMINATION:-100000000}" \
    ${MAX_WITHDRAWAL_AMOUNT:+--max-withdrawal-amount "$MAX_WITHDRAWAL_AMOUNT"} \
    --txpool.minimal-protocol-fee "${TXPOOL_MIN_PROTOCOL_FEE:-0}" \
    --genesis-l1-height "${GENESIS_L1_HEIGHT:?GENESIS_L1_HEIGHT must be set}" \
    "$@"
