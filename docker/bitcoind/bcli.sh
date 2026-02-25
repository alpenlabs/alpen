#!/bin/bash
set -euo pipefail

BITCOIN_NETWORK="${BITCOIN_NETWORK:-regtest}"

bitcoin-cli \
    -"${BITCOIN_NETWORK}" \
    -rpcuser="${BITCOIND_RPC_USER}" \
    -rpcpassword="${BITCOIND_RPC_PASSWORD}" \
    "$@"
