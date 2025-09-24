#!/bin/bash
# Usage: ./init-keys.sh <path_to_datatool_binary>
#
# Bitcoin RPC credentials can be provided via environment variables:
#   BITCOIN_RPC_URL - Bitcoin RPC URL (default: http://localhost:18443)
#   BITCOIN_RPC_USER - Bitcoin RPC username (default: rpcuser)
#   BITCOIN_RPC_PASSWORD - Bitcoin RPC password (default: rpcpassword)
#
# Or passed as additional arguments after the datatool path

DATATOOL_PATH=${1:-./strata-datatool}
shift

# Set default Bitcoin RPC credentials if not provided
BITCOIN_RPC_URL=${BITCOIN_RPC_URL:-"http://localhost:18443"}
BITCOIN_RPC_USER=${BITCOIN_RPC_USER:-"rpcuser"}
BITCOIN_RPC_PASSWORD=${BITCOIN_RPC_PASSWORD:-"rpcpassword"}

echo "Checking if 'base58' is installed.".
if ! command -v base58 &> /dev/null; then \
	echo "base58 not found. Please install with 'pip install base58'." \
	exit 1; \
fi

CONFIG_FILE=configs

JWT_FILE=$CONFIG_FILE/jwt.hex
JWT_FN_FILE=$CONFIG_FILE/jwt.fn.hex

generate_random_hex() {
    if [ -z "$1" ]; then
        return 1
    fi

    if [ -e "$1" ]; then
        echo "File '$1' already exists. Skipping."
        return 0
    fi

    # Generate 32 random bytes, convert to hex, and write to the file
    od -An -tx1 -N32 /dev/urandom | tr -d ' \n' > "$1"
}

generate_random_hex $JWT_FILE
generate_random_hex $JWT_FN_FILE

SEQ_SEED_FILE=$CONFIG_FILE/sequencer.bin
OP1_SEED_FILE=$CONFIG_FILE/operator1.bin
OP2_SEED_FILE=$CONFIG_FILE/operator2.bin
OP3_SEED_FILE=$CONFIG_FILE/operator3.bin
OP4_SEED_FILE=$CONFIG_FILE/operator4.bin
OP5_SEED_FILE=$CONFIG_FILE/operator5.bin

$DATATOOL_PATH -b regtest genxpriv -f $SEQ_SEED_FILE
$DATATOOL_PATH -b regtest genxpriv -f $OP1_SEED_FILE
$DATATOOL_PATH -b regtest genxpriv -f $OP2_SEED_FILE
$DATATOOL_PATH -b regtest genxpriv -f $OP3_SEED_FILE
$DATATOOL_PATH -b regtest genxpriv -f $OP4_SEED_FILE
$DATATOOL_PATH -b regtest genxpriv -f $OP5_SEED_FILE

seqprivkey=$($DATATOOL_PATH -b regtest genseqprivkey -f ${SEQ_SEED_FILE})
echo -n "$seqprivkey" > $CONFIG_FILE/sequencer.key

op1xpriv=$(cat $OP1_SEED_FILE)
op2xpriv=$(cat $OP2_SEED_FILE)
# shellcheck disable=2034
op3xpriv=$(cat $OP3_SEED_FILE)
# shellcheck disable=2034
op4xpriv=$(cat $OP4_SEED_FILE)
# shellcheck disable=2034
op5xpriv=$(cat $OP5_SEED_FILE)

seqpubkey=$($DATATOOL_PATH -b regtest genseqpubkey -f ${CONFIG_FILE}/sequencer.key)

ROLLUP_PARAMS_FILE=$CONFIG_FILE/params.json

# Construct args for genparams.
# Check if -n is set in args
# shellcheck disable=2199
if [[ "$@" != *"-n "* ]]; then
    extra_args+=("-n" "alpn")
fi

if [ -z "$output_found" ]; then
    extra_args+=(--output "$ROLLUP_PARAMS_FILE")
fi

# Add Bitcoin RPC credentials to genparams command
$DATATOOL_PATH -b regtest \
    --bitcoin-rpc-url "$BITCOIN_RPC_URL" \
    --bitcoin-rpc-user "$BITCOIN_RPC_USER" \
    --bitcoin-rpc-password "$BITCOIN_RPC_PASSWORD" \
    genparams \
    -s "$seqpubkey" \
    -b "$op1xpriv" \
    -b "$op2xpriv" \
    "${extra_args[@]}" \
    "$@"
