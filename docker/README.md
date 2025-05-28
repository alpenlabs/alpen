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

```bash
# build the datatool
cargo build --bin strata-datatool
cd docker
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

## Prover Client

> Before proceeding, make sure that all of the prerequisites listed above have been met.

1. Build the datatool in `sp1` mode:

    ```bash
    cargo build --bin strata-datatool -F "sp1-builder" --release
    ```

2. Export the generated ELF

    ```bash
    target/release/strata-datatool genparams --elf-dir docker/prover-client/elfs/sp1 
    ```

3. Generate configs

    ```bash
    cd docker && ./init-keys.sh ../target/release/strata-datatool
    ```

4. Run the prover-client

    ```bash
    rm -rf .data && docker compose up prover-client
    ```
