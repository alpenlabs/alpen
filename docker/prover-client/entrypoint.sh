#!/bin/bash

# Exit on error
set -e

echo "starting Prover client"

# Set default config path
CONFIG_PATH=${CONFIG_PATH:-/app/prover-client.toml}

# Check if config file exists, if not use the sample
if [ ! -f "$CONFIG_PATH" ]; then
    echo "Config file not found at $CONFIG_PATH, using sample config"
    CONFIG_PATH="/app/prover-client.sample.toml"
fi

# Start the prover client with config file
# Note: Command line arguments will override config file values
strata-prover-client \
    --config "$CONFIG_PATH" \
    $@
