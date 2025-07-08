#!/bin/bash
# Usage: ./init-keys.sh <path_to_datatool_binary>
DATATOOL_PATH=${1:-./strata-datatool}
shift

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
echo -n $seqprivkey > $CONFIG_FILE/sequencer.key

op1pubkey=$($DATATOOL_PATH -b regtest genopxpub -f ${OP1_SEED_FILE})
op2pubkey=$($DATATOOL_PATH -b regtest genopxpub -f ${OP2_SEED_FILE})
op3pubkey=$($DATATOOL_PATH -b regtest genopxpub -f ${OP3_SEED_FILE})
op4pubkey=$($DATATOOL_PATH -b regtest genopxpub -f ${OP4_SEED_FILE})
op5pubkey=$($DATATOOL_PATH -b regtest genopxpub -f ${OP5_SEED_FILE})

seqpubkey=$($DATATOOL_PATH -b regtest genseqpubkey -f ${CONFIG_FILE}/sequencer.key)

ROLLUP_PARAMS_FILE=$CONFIG_FILE/params.json

# Construct args for genparams.
# Check if -n is set in args
if [[ "$@" != *"-n "* ]]; then
    extra_args+=("-n" "alpenstrata")
fi

# Check if --output is set
if [[ "$@" != *"--output "* ]]; then
    extra_args+=(--output "$ROLLUP_PARAMS_FILE")
fi

$DATATOOL_PATH -b regtest genparams \
    -s $seqpubkey \
    -b $op1pubkey \
    -b $op2pubkey \
    "${extra_args[@]}" \
    "$@"
