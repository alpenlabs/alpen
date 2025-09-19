#!/bin/bash

bitcoin-cli -regtest -rpcuser="${BITCOIND_RPC_USER}" -rpcpassword="${BITCOIND_RPC_PASSWORD}" "$@"
