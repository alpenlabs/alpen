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
    # TODO: add conditional builds as we go
    # TODO: different binaries for sequencer and full nodes
    cargo build  -F sequencer -F debug-utils -F test-mode -F debug-asm --bin strata --bin alpen-client --bin strata-datatool --bin strata-test-cli --bin strata-dbtool
}

# Ensures upstream reth binary is available (needed for init-state tests).
ensure_reth() {
    if ! command -v reth &> /dev/null; then
        echo "Installing reth v1.9.1 (needed for init-state tests)..."
        cargo install reth --git https://github.com/paradigmxyz/reth --tag v1.9.1 --locked
    fi
}

# Runs tests.
run_tests() {
    uv sync
    uv run entry.py "$@"
}

setup_path
build
ensure_reth
run_tests "$@"
