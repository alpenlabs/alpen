# Running Locally

## Quick Start

```bash
# Copy and configure .env (set MNEMONIC for signet miner):
cp .env.example .env

# Start everything:
just docker-seq-up

# Stop everything:
just docker-seq-down
```

## Architecture

| Compose | Services |
|---|---|
| `compose-signet.yml` | `bitcoind` (local signet miner or fullnode) |
| `compose-ol-el-seq.yml` | `init` → `strata` → `strata-signer`, `alpen-client` |

Bitcoin is decoupled from the sequencer stack. `init` waits for bitcoin to reach `GENESIS_L1_HEIGHT`, generates params, then exits. Other services start after.

## Just Recipes

| Recipe | Description |
|---|---|
| `just docker-seq-up` | Start signet + sequencer stack |
| `just docker-seq-down` | Stop everything |
| `just docker-signet-up` | Start signet only |
| `just docker-signet-down` | Stop signet only |
| `just docker-seq-build` | Rebuild sequencer images |

## Without Just

For controlled image builds, step-by-step debugging, or running individual services, see the just recipes in `.justfile` (search for `group('docker')`) for the underlying `docker compose` commands.

## With remote Bitcoin

Set `BITCOIND_RPC_URL` in `.env` to the remote endpoint and run `just docker-seq-up` as usual. The init service connects to whatever `BITCOIND_RPC_URL` points to.
