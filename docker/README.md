# Running Locally

The [docker-compose](./docker-compose.yml) file is meant for local dev environments
(although it could just as easily be used in production environments as well with the right setup).
In order to run the containers locally, you can follow the instructions below,
which also includes some details on the necessary non-docker pre-setup.

## Pre-requisites

1. Install `base58` on your system:

    ```python
    pip3 install base58
    ```

1. Install `Docker Desktop` on your machine (Windows, Mac) or install `docker` (Linux).

1. If you are running the prover client, ensure that the `docker/prover-client/prover-client.env` file is present. For reference, refer to `prover-client.sample.env`

## Running

Generate the required keys:

**NOTE**: Datatool requires a connection to bitcoin network to generate params. If the bitcoin network is other than regtest, make sure it is set to the environment variable `BITCOIN_NETWORK`.

```bash
# build the datatool
cargo build --bin strata-datatool
cd docker

# Bitcoin RPC connection is required. Credentials can be provided via environment variables:
# If not set, defaults to: http://localhost:18443, rpcuser, rpcpassword
export BITCOIN_RPC_URL="http://localhost:18443"
export BITCOIN_RPC_USER="rpcuser"
export BITCOIN_RPC_PASSWORD="rpcpassword"

./init-keys.sh <path_to_strata_datatool> # typically, ../target/debug/strata-datatool
```

The above step should create root xprivs in the [`docker/configs`](./configs) directory.
Build and run the containers:

```bash
docker compose up --build
```

Chances are that the above step will fail as some bitcoin blocks have to be mined before the `strata_client` container can work properly.
Mining of the required number of blocks should happen automatically when the `stata_bitcoind` container starts.
After that, you can simply restart the containers:

```bash
docker start strata_sequencer
docker start alpen_reth_fn # if you want to test the full node
```

## Prover Client (with SP1)

> Before proceeding, make sure that all of the prerequisites listed above have been met.

1. Build the datatool with `sp1-builder` and `btc-client` features:

    ```bash
    cargo build --bin strata-datatool -F "sp1-builder,btc-client" --release
    ```

    **Note**: Both features are required:
    - `sp1-builder`: Compiles guest programs and generates ELF files with VKs
    - `btc-client`: Enables Bitcoin RPC connectivity for `genparams` command

2. Export guest ELF files:

    ```bash
    # Set Bitcoin RPC credentials (required by genparams)
    export BITCOIN_NETWORK=signet  # or "regtest"
    export BITCOIN_RPC_URL="http://<rpc-endpoint>"
    export BITCOIN_RPC_USER="<user>"
    export BITCOIN_RPC_PASSWORD="<password>"

    # Export compiled guest ELF files to docker directory
    target/release/strata-datatool -b "$BITCOIN_NETWORK" \
      --bitcoin-rpc-url "$BITCOIN_RPC_URL" \
      --bitcoin-rpc-user "$BITCOIN_RPC_USER" \
      --bitcoin-rpc-password "$BITCOIN_RPC_PASSWORD" \
      genparams --elf-dir docker/prover-client/elfs/sp1
    ```

    **Note**: This step compiles guest programs and exports ELF files to `docker/prover-client/elfs/sp1/`. These ELF files will be copied into the Docker image and loaded by the prover-client at runtime using the `ELF_BASE_PATH` environment variable.

3. Generate configs, keys, and params:

    ```bash
    # The --chain-config argument allows switching between different chainspecs during deployment
    # Available chainspecs: crates/reth/chainspec/src/res/{alpen-dev-chain.json, devnet-chain.json, testnet-chain.json (default)}
    cd docker && ./init-keys.sh ../target/release/strata-datatool --chain-config ../crates/reth/chainspec/src/res/testnet-chain.json
    ```

    **Note**: This step generates JWT tokens, sequencer/operator keys, and `params.json`. Since the datatool was built with `sp1-builder` feature, the verification keys in `params.json` will match the ELF files exported in step 2 (both come from the same build).

4. Run the prover-client

    ```bash
    rm -rf .data && docker compose up prover-client
    ```

## Troubleshooting
- If you get an error `ERROR Max retries 3 exceeded` while running `init-keys.sh`, make sure the bitcoin node's endpoint provided via `BITCOIN_RPC_URL` is running.

## Running OL + EL with the shared compose

This setup builds both `strata` (OL) and `alpen-client` (EL) from the repo Dockerfiles and wires them to a local bitcoind.

1. Ensure config files exist:
   - `docker/configs/config.toml`
   - `docker/configs/params.json`
2. From repo root, start the stack:
   ```bash
   docker compose -f docker/docker-compose-ol-el.yml up -d --build
   ```
3. Services:
   - `strata` listens on `8432` and reads the mounted config/params at `/config/config.toml` and `/config/params.json`.
   - `alpen-client` exposes `8545/8546/30303` and is pointed at `http://strata:8432`.
4. Stop the stack when done:
   ```bash
   docker compose -f docker/docker-compose-ol-el.yml down
   ```

### Run OL only (silo)
- Prereq: bitcoind must be up; compose handles it automatically.
- Start only OL + bitcoind:
  ```bash
  docker compose -f docker/docker-compose-ol-el.yml up -d --build bitcoind strata
  ```
- Stop:
  ```bash
  docker compose -f docker/docker-compose-ol-el.yml down
  ```

### Run EL only (silo)
- Assumes an OL endpoint is available at `--ol-client-http` (update the command in `docker/docker-compose-ol-el.yml` if needed).
- Start only EL:
  ```bash
  docker compose -f docker/docker-compose-ol-el.yml up -d --build alpen-client
  ```
- Stop:
  ```bash
  docker compose -f docker/docker-compose-ol-el.yml down
  ```
