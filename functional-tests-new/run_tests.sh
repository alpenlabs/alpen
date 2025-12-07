#!/bin/bash
set -e

export RUST_BACKTRACE=1

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
    cargo build  -F debug-utils -F test-mode --bin strata
}

# Runs tests.
run_tests() {
    uv sync
    uv run entry.py "$@"
}

setup_path
build
run_tests "$@"
