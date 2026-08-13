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
| `compose-ol-seq.yml` | OL sequencer and external `strata-signer` |

Bitcoin is decoupled from the OL stack. `just docker-seq-up` starts signet, runs `gen-params-and-elfs.sh`, then starts the sequencer stack. Generated keys and params live under `configs/generated/` and are ignored by git.

The EE node (`alpen-client`) and its compose files live in the [alpen-ee](https://github.com/alpenlabs/alpen-ee) repo.

The external `strata-signer` reads the sequencer admin bearer token from
`STRATA_ADMIN_RPC_TOKEN`, so deployments do not need to hardcode that secret in
the signer config TOML.

The retained secondary compose file has a narrower test/debug purpose:

| Compose | Purpose |
|---|---|
| `compose-checkpoint-sync.yml` | Checkpoint-sync OL node; use with a signet fullnode and mount pre-generated params under `configs/generated/` |

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
