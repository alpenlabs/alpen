#!/bin/bash

# Usage: ./bitcoin_block_gen.sh <timeout>

# Helper script to periodically generate bitcoin blocks inside docker container `bitcoind`
# @author: sapinb

generateblock_time() {
   block_interval_sec=${1:-60}

   echo Generate blocks every "$block_interval_sec" Sec
   while true; do
      sleep "$block_interval_sec"
      docker compose exec bitcoind /app/bcli.sh -rpcwallet=default -generate 1
   done
}

generateblock_time "$@"
