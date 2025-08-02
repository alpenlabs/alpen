#!/bin/bash
set -e

source env.bash

if [ "$CARGO_RELEASE" = "1" ]; then
    export PATH=$(realpath ../target/release/):$PATH
else
    export PATH=$(realpath ../target/debug/):$PATH
fi

# Conditionally run cargo build based on PROVER_TEST
if [ ! -z "$PROVER_TEST" ]; then
    echo "Running on sp1-builder mode"
    cargo build --release -F sp1-builder
    export PATH=$(realpath ../target/release/):$PATH
elif [ ! -z "$CI_COVERAGE" ]; then
    echo "Running strata client with coverage"
    # same targe dir and coverage format as cargo-llvm-cov
    COV_TARGET_DIR=$(realpath ../target)"/llvm-cov-target"
    mkdir -p "$COV_TARGET_DIR"
    export LLVM_PROFILE_FILE=$COV_TARGET_DIR"/strata-%p-%m.profraw"
    RUSTFLAGS="-Cinstrument-coverage" cargo build -F debug-utils --target-dir "$COV_TARGET_DIR"
    export PATH=$COV_TARGET_DIR/debug:$PATH
else
    echo "Running strata client"
    cargo build -F debug-utils
fi

poetry run python entry.py "$@"
