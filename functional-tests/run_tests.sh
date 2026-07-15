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

# Builds the alpen binaries from this workspace.
build() {
    # TODO(STR-3692): different binaries for sequencer and full nodes
    cargo build --bin alpen-client
}

# Builds the strata binaries from the git rev pinned in the root Cargo.toml
# and puts them on PATH ahead of any stale workspace-built ones.
build_strata() {
    local strata_bin_dir
    strata_bin_dir="$(./build_strata_bins.sh)"
    export PATH="$strata_bin_dir:$PATH"
}

# Runs tests.
run_tests() {
    uv sync
    uv run entry.py "$@"
}

setup_path
build
build_strata
run_tests "$@"
