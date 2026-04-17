#!/bin/bash
set -e

export RUST_BACKTRACE=1
export RUST_LOG="debug,sled=warn,hyper=warn,h2=warn,soketto=warn,jsonrpsee-server=warn,mio=warn"

# Sets up PATH for built binaries.
setup_path() {
    if [ "$CARGO_RELEASE" = 1 ]; then
      # shellcheck disable=2155
      export PATH=$(realpath ../target/release/):$PATH
    else
      # shellcheck disable=2155
      export PATH=$(realpath ../target/debug/):$PATH
    fi
}

# Builds the binary.
build() {
    # TODO(STR-3692): add conditional builds as we go
    # TODO(STR-3692): different binaries for sequencer and full nodes
    cargo build  -F sequencer -F debug-utils -F test-mode -F debug-asm -F prover --bin strata --bin strata-signer --bin alpen-client --bin strata-datatool --bin strata-test-cli --bin strata-dbtool --bin alpen-ee-da-tool
}

# Runs tests.
run_tests() {
    uv sync
    uv run entry.py "$@"
}

setup_path
build
run_tests "$@"
