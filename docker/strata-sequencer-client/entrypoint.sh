#!/bin/bash -e

export RUST_LOG=${RUST_LOG:-info}
export RUST_BACKTRACE=${RUST_BACKTRACE:-full}

# Start the strata sequencer signer
strata-sequencer-client "$@"
