#!/bin/bash
set -euo pipefail

BITCOIN_NETWORK="${BITCOIN_NETWORK:-regtest}"

case "${BITCOIN_NETWORK}" in
    regtest)
        CHAIN_FLAG="-regtest"
        ;;
    signet)
        CHAIN_FLAG="-signet"
        ;;
    *)
        echo "error: unsupported BITCOIN_NETWORK=${BITCOIN_NETWORK}" >&2
        exit 1
        ;;
esac

# generate bitcoin.conf
cat > /root/.bitcoin/bitcoin.conf <<EOF
${BITCOIN_NETWORK}=1

[${BITCOIN_NETWORK}]
rpcuser=${BITCOIND_RPC_USER}
rpcpassword=${BITCOIND_RPC_PASSWORD}
rpcbind=0.0.0.0
rpcallowip=${RPC_ALLOW_IP:-0.0.0.0/0}
server=1
txindex=1
fallbackfee=0.00001
EOF

# regtest-only: allow burning and non-standard txs
if [ "${BITCOIN_NETWORK}" = "regtest" ]; then
    cat >> /root/.bitcoin/bitcoin.conf <<EOF
maxburnamount=1
acceptnonstdtxn=1
EOF
fi

bcli() {
    bitcoin-cli "${CHAIN_FLAG}" \
        -rpcuser="${BITCOIND_RPC_USER}" \
        -rpcpassword="${BITCOIND_RPC_PASSWORD}" \
        "$@"
}

# start bitcoind in the background
bitcoind -conf=/root/.bitcoin/bitcoin.conf "${CHAIN_FLAG}" "$@" &

echo "Waiting for bitcoind (${BITCOIN_NETWORK}) to be ready..."
for _ in $(seq 1 30); do
    if bcli getblockchaininfo >/dev/null 2>&1; then
        echo "bitcoind started"
        break
    fi
    sleep 1
done

if ! bcli getblockchaininfo >/dev/null 2>&1; then
    echo "error: bitcoind did not start" >&2
    exit 1
fi

# create wallets, mainly for docker cache
for WALLET in ${BITCOIND_WALLET:-default}; do
    if bcli listwalletdir | grep -q "\"name\": \"${WALLET}\""; then
        bcli loadwallet "${WALLET}" 2>/dev/null || true
    else
        bcli -named createwallet wallet_name="${WALLET}" descriptors=true
    fi
done

if [ "${BITCOIN_NETWORK}" = "regtest" ]; then
    BLOCK_COUNT=$(bcli getblockcount)
    if [ "${BLOCK_COUNT}" -eq 0 ]; then
        ADDRESS=$(bcli -rpcwallet="${BITCOIND_WALLET:-default}" getnewaddress)
        # 101 to mature coinbase + a few more for rollup genesis
        echo "Generating 120 initial blocks to ${ADDRESS}..."
        bcli generatetoaddress 120 "${ADDRESS}"
    fi

    # generate single blocks
    if [ -n "${GENERATE_BLOCKS:-}" ]; then
        ADDRESS=$(bcli -rpcwallet="${BITCOIND_WALLET:-default}" getnewaddress)
        while true; do
            bcli generatetoaddress 1 "${ADDRESS}"
            sleep "${GENERATE_BLOCKS}"
        done
    fi
fi

wait -n
exit $?
