#!/bin/sh

docker compose exec bitcoind /app/bcli.sh "$@"

# helpful commands
# ./bcli.sh -generate 5
