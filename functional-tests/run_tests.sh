#!/bin/bash
set -e

cd "$(dirname "$0")"

export RUST_BACKTRACE=1
export RUST_LOG="debug,sled=warn,hyper=warn,h2=warn,soketto=warn,jsonrpsee-server=warn,mio=warn"
# Service logs are stdout captures, and tracing colours stdout by default -- including
# between a field name and its value. Tests that grep those logs then silently never
# match. Keep the captured logs plain.
export NO_COLOR=1

# Native is the ordinary functional-test backend. SP1 runs this upgrade test
# against the prover network, with the frozen V1 bundle and a freshly built V2.
OL_PROVER_BACKEND="${OL_PROVER_BACKEND:-native}"
case "$OL_PROVER_BACKEND" in
  native) ;;
  sp1)
    if [ "${SP1_PROVER:-}" != network ]; then
      echo "OL_PROVER_BACKEND=sp1 requires SP1_PROVER=network" >&2
      exit 1
    fi
    if [ -z "${NETWORK_PRIVATE_KEY:-}" ]; then
      echo "SP1 network proving requires NETWORK_PRIVATE_KEY" >&2
      exit 1
    fi
    ;;
  *)
    echo "Unknown OL_PROVER_BACKEND: $OL_PROVER_BACKEND (expected: native|sp1)" >&2
    exit 1
    ;;
esac
export OL_PROVER_BACKEND

# Sets up PATH for built binaries, including custom Cargo target directories.
setup_path() {
    local target_dir="${CARGO_TARGET_DIR:-../target}"
    local profile=debug
    if [ "${CARGO_RELEASE:-}" = 1 ] || [ "$OL_PROVER_BACKEND" = sp1 ]; then
      profile=release
    fi
    case "$target_dir" in
      /*) ;;
      *) target_dir="$PWD/$target_dir" ;;
    esac
    export PATH="$target_dir/$profile:$PATH"
}

# Builds the node and tools used by functional tests.
build() {
    local args=(--locked -F sequencer -F debug-utils -F prover)
    if [ "$OL_PROVER_BACKEND" = sp1 ]; then
      args+=(--release -F sp1)
    elif [ "${CARGO_RELEASE:-}" = 1 ]; then
      args+=(--release)
    fi
    cargo build "${args[@]}" --bin strata --bin strata-signer --bin strata-datatool --bin strata-test-cli --bin strata-dbtool
}

# Builds V2 with the same runtime parameters that are embedded in frozen V1.
# The build script derives the predicate and artifact manifest from that ELF.
build_sp1_guest() {
    [ "$OL_PROVER_BACKEND" = sp1 ] || return 0
    BUILD_VKEY=1 SP1_SKIP_PROGRAM_BUILD=false \
      CHECKPOINT_RUNTIME_PARAMS_PATH="$PWD/testdata/sp1/v1/checkpoint-runtime-params.json" \
      cargo build --locked --release -p strata-sp1-guest-builder
}

# Runs only the tests selected by the caller.
run_tests() {
    uv sync
    uv run entry.py "$@"
}

setup_path
build
build_sp1_guest
run_tests "$@"
