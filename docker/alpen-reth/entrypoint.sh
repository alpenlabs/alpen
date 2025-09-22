#! /bin/bash

## Entry point for Dockerfile

# Exit on error
set -e

mkdir -p /app/reth

echo "starting Reth"

alpen-reth \
    --disable-discovery \
    --datadir "${DATADIR:-/app/reth}" \
    --port 30303 \
    --p2p-secret-key "${P2p_SECRET_KEY:-p2p.hex}" \
    --authrpc.addr 0.0.0.0 \
    --authrpc.port 8551 \
    --authrpc.jwtsecret "${JWTSECRET:-jwt.hex}" \
    --http \
    --http.addr 0.0.0.0 \
    --http.port 8545 \
    --http.api "${HTTP_API:-eth,net,web3,txpool}" \
    --ws \
    --ws.addr 0.0.0.0 \
    --ws.port 8546 \
    --ws.api "${WS_API:-eth,net,web3,txpool}" \
    --color never \
    -vvvv "$@" \
