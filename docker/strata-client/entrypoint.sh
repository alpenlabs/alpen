#! /bin/bash
set -e

error=0

CONFIG_PATH=${CONFIG_PATH:-config.toml}
PARAM_PATH=${PARAM_PATH:-params.json}

if [ ! -f "$CONFIG_PATH" ]; then
    echo  "Error: Missing config file '$CONFIG_PATH'."
    error=1
fi

if [ ! -f "$PARAM_PATH" ]; then
    echo  "Error: Missing params file '$PARAM_PATH'."
    error=1
fi

if [[ $error -ne 0 ]]; then
    exit 1
fi

ARGS=""

# if [ ! -z "$BITCOIND_PASSWORD" ]; then
#     ARGS="$ARGS --bitcoind-password $BITCOIND_PASSWORD"
# fi

export RUST_LOG=${RUST_LOG:-info}

# Start the Strata Client
strata-client --config "$CONFIG_PATH" --rollup-params "$PARAM_PATH" "$@"
