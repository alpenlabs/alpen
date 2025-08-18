#! /bin/bash
set -e

filepath=/Users/cdjk/github/alpen/vertex-core/crates/util/python-utils
cp $filepath/pyproject.toml $filepath/pyproject.toml.backup && \
gawk '/version.*-pre\.[0-9]+/ { match($0, /-pre\.([0-9]+)/, a); gsub(/-pre\.[0-9]+/, "-pre." (a[1]+1)); print "Changed -pre." a[1] " to -pre." (a[1]+1) > "/dev/stderr" } 1' $filepath/pyproject.toml > temp && mv temp $filepath/pyproject.toml

poetry update
source env.bash

if [ "$CARGO_RELEASE" = 1 ]; then
	export PATH=$(realpath ../target/release/):$PATH
else
	export PATH=$(realpath ../target/debug/):$PATH
fi

# Conditionally run cargo build based on PROVER_TEST
if [ ! -z $PROVER_TEST ]; then
    echo "Running on sp1-builder mode"
    cargo build --release -F sp1-builder
	export PATH=$(realpath ../target/release/):$PATH
else
    echo "Running strata client"
    cargo build -F debug-utils
fi

poetry run python entry.py $@
