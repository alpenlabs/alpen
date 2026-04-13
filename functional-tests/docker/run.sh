#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Build if image doesn't exist or --build flag passed
if [ "$1" = "--build" ]; then
    shift
    docker compose build
fi

docker compose run --rm functional-tests "$@"
