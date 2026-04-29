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

exec alpen-client \
    --sequencer \
    --sequencer-pubkey "${SEQUENCER_PUBKEY}" \
    --ol-client-url "${OL_CLIENT_URL:-ws://strata:8432}" \
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
    "$@"
