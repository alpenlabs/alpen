# Docker

## Quick Start

```bash
# Copy and configure developer inputs. Set MNEMONIC for local signet mining.
cp .env.example .env

just docker-seq-up
just docker-seq-down
```

## Architecture

The primary local stack is split into two compose files:

| Compose | Purpose |
|---|---|
| `compose-signet.yml` | Local signet `bitcoind` miner or fullnode |
| `compose-ol-el-seq.yml` | OL sequencer, external `strata-signer`, and EE sequencer |

Bitcoin is decoupled from the OL/EE stack. `just docker-seq-up` starts signet, runs `gen-params-and-elfs.sh`, then starts the sequencer stack. Generated keys, params, and env files live under `configs/generated/` and are ignored by git.

The external `strata-signer` reads the sequencer admin bearer token from
`STRATA_ADMIN_RPC_TOKEN`, so deployments do not need to hardcode that secret in
the signer config TOML.

The retained secondary compose files have narrower test/debug purposes:

| Compose | Purpose |
|---|---|
| `compose-fullnode.yml` | Alpen EE fullnode |
| `compose-checkpoint-sync.yml` | Checkpoint-sync OL node; use with a signet fullnode and mount pre-generated params under `configs/generated/` |
| `docker-compose-eest.yml` | Ethereum execution spec test environment |
| `docker-compose-p2p-test.yml` | Minimal EE P2P/gossip test |

For operator-style fullnode validation, use `compose-fullnode.yml`. By default
it runs the Alpen EE fullnode against the `OL_CLIENT_URL` configured in `.env`.

Create the fullnode environment file before starting that stack:

```bash
cp .env.alpen-fullnode.example .env
# Edit .env for the target network images and Alpen EE peers.
```

The fullnode compose mounts `configs/ee-params.testnet3.json`, which contains
the Testnet III EE genesis metadata and bridge params consumed by
`alpen-client --ee-params`.

The compose defaults HTTP RPC to `eth,net,web3,txpool`; it does not expose
`admin` or `debug` unless the operator explicitly overrides `HTTP_API`.

Prepare the required key file mounted by the Alpen fullnode service:

```bash
mkdir -p configs/generated

openssl rand -hex 32 > configs/generated/jwt.hex
chmod 644 configs/generated/jwt.hex
```

To build the fullnode image from this checkout, set a local image name in
`.env`:

```bash
ALPEN_IMAGE=alpen-client:local
```

Then build the image:

```bash
docker compose -f compose-fullnode.yml build alpen-fullnode
```

Then start the Alpen fullnode:

```bash
docker compose -f compose-fullnode.yml up -d
```

## Just Recipes

| Recipe | Description |
|---|---|
| `just docker-seq-up` | Start signet + sequencer stack |
| `just docker-seq-down` | Stop everything |
| `just docker-signet-up` | Start signet only |
| `just docker-signet-down` | Stop signet only |
| `just docker-seq-build` | Rebuild sequencer images |

## Without Just

For controlled image builds, step-by-step debugging, or running individual services, use the commands behind the just recipes in `.justfile` under `group('docker')`.

## With remote Bitcoin

Set `BITCOIND_RPC_URL` in `.env` to the remote endpoint and run `just docker-seq-up` as usual. The init service connects to whatever `BITCOIND_RPC_URL` points to.
