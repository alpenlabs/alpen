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

The retained secondary compose files have narrower test/debug purposes:

| Compose | Purpose |
|---|---|
| `compose-checkpoint-sync.yml` | Checkpoint-sync OL node — depends only on a bitcoin chain; mount pre-generated params under `configs/sync-params/` |
| `docker-compose-eest.yml` | Ethereum execution spec test environment |
| `docker-compose-p2p-test.yml` | Minimal EE P2P/gossip test |

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
