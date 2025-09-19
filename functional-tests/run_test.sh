#! /bin/bash
set -e

cd "$(dirname "$0")"
# shellcheck disable=SC1091
source env.bash

if [ "$CARGO_RELEASE" = 1 ]; then
  # shellcheck disable=2155
  export PATH=$(realpath ../target/release/):$PATH
else
  # shellcheck disable=2155
  export PATH=$(realpath ../target/debug/):$PATH
fi

# Conditionally run cargo build based on PROVER_TEST
if [ -n "$PROVER_TEST" ]; then
  echo "Running on sp1-builder mode"
  cargo build --release -F sp1-builder
  # shellcheck disable=2155
  export PATH=$(realpath ../target/release/):$PATH
elif [ -n "$CI_COVERAGE" ]; then
  echo "Running strata client with coverage"
  # same targe dir and coverage format as cargo-llvm-cov
  COV_TARGET_DIR=$(realpath ../target)"/llvm-cov-target"
  mkdir -p "$COV_TARGET_DIR"
  export LLVM_PROFILE_FILE=$COV_TARGET_DIR"/strata-%p-%m.profraw"
  RUSTFLAGS="-Cinstrument-coverage" cargo build -F debug-utils -F test-mode --target-dir "$COV_TARGET_DIR"
  export PATH=$COV_TARGET_DIR/debug:$PATH
else
  echo "Building and running strata client"
  if [ "$DB_BACKEND" = "rocksdb" ]; then
    cargo build -F debug-utils -F test-mode -F rocksdb
    echo built with rocksdb
  else
    cargo build -F debug-utils -F test-mode
    echo built with default sled
  fi
fi

uv run python entry.py "$@"
