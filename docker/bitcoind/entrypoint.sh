#! /bin/bash -x
BITCOIND_CONF_DIR=/home/bitcoin

# Generate bitcoin.conf
cat <<EOF > ${BITCOIND_CONF_DIR}/bitcoin.conf
regtest=1

[regtest]
rpcuser=${BITCOIND_RPC_USER}
rpcpassword=${BITCOIND_RPC_PASSWORD}
rpcbind=0.0.0.0
rpcallowip=${RPC_ALLOW_IP}
fallbackfee=0.00001
maxtxfee=10000
maxfeerate=1000000
maxburnamount=1
server=1
txindex=1
acceptnonstdtxn=1
printtoconsole=1
acceptnonstdtxn=1
minrelaytxfee=0.0
blockmintxfee=0.0
dustRelayFee=0.0
debug=zmq
zmqpubhashblock=tcp://0.0.0.0:28332
zmqpubhashtx=tcp://0.0.0.0:28333
zmqpubrawblock=tcp://0.0.0.0:28334
zmqpubrawtx=tcp://0.0.0.0:28335
zmqpubsequence=tcp://0.0.0.0:28336
EOF

echo "Bitcoin RPC User: $BITCOIND_RPC_USER"

bcli() {
    bitcoin-cli -regtest -rpcuser=${BITCOIND_RPC_USER} -rpcpassword=${BITCOIND_RPC_PASSWORD} $@
}

# Start bitcoind in the background
bitcoind -conf=$BITCOIND_CONF_DIR/bitcoin.conf -regtest &

# Function to check if a wallet exists and is loaded, mainly for docker cache
check_wallet_exists() {
  echo "Checking if wallet '$1' exists in the wallet directory..."

  # List all wallets in the wallet directory
  ALL_WALLETS=$(bcli listwalletdir)

  echo $ALL_WALLETS

  # Check if the wallet name is in the list of wallets in the directory
  if echo "$ALL_WALLETS" | grep -q "\"name\": \"${1}\""; then
    echo "Wallet '$1' exists in the wallet directory."
    bcli loadwallet $1
  else
    echo "Wallet '$1' does not exist in the wallet directory."
    bcli -named createwallet wallet_name="${1}" descriptors=true
    bcli loadwallet $1
  fi

  return 0
}

# Function to check if bitcoind is ready
wait_for_bitcoind() {
  echo "Waiting for bitcoind to be ready..."
  for i in $(seq 1 10); do
    result=$(bcli getblockchaininfo 2>/dev/null)
    if [ $? -eq 0 ]; then
      echo "Bitcoind started"
      return 0
    else
      sleep 1
    fi
  done
  return 1
}

# Wait until bitcoind is fully started
wait_for_bitcoind

if [ $? -eq 1 ]; then
    echo "Bitcoin didn't start properly. Exiting"
    exit
fi

# create wallet
check_wallet_exists $BITCOIND_WALLET

VAL=$(bitcoin-cli getblockcount)

bcli generatetoaddress 1 $GENERAL_WALLET_1
bcli generatetoaddress 1 $GENERAL_WALLET_2
bcli generatetoaddress 1 $GENERAL_WALLET_3

if [[ $VAL -eq 0 ]]; then
    # Get a new Bitcoin address from the wallet
    ADDRESS=$(bcli -rpcwallet="${BITCOIND_WALLET}" getnewaddress)

    echo "Generated new address: $ADDRESS"
    echo $ADDRESS > $BITCOIND_CONF_DIR/bitcoin-address

    # Generate 101 blocks to the new address
    # to mature coinbase funds
    echo "Generating 120 blocks..."
    bcli generatetoaddress 105 "$ADDRESS"
fi

bcli sendtoaddress $STAKE_CHAIN_WALLET_1 $CLAIM_FUNDING_AMOUNT
bcli sendtoaddress $STAKE_CHAIN_WALLET_2 $CLAIM_FUNDING_AMOUNT
bcli sendtoaddress $STAKE_CHAIN_WALLET_3 $CLAIM_FUNDING_AMOUNT

# generate single blocks
if [ ! -z $GENERATE_BLOCKS ];then
while :
do
    bcli generatetoaddress 1 "$ADDRESS"
    sleep $GENERATE_BLOCKS
done
else
    wait -n
    exit $?
fi

