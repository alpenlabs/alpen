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

## ASM execution parameters

Node TOML requires `asm_execution = "asm-execution-params.json"`. Relative paths
resolve against the TOML file's directory. The referenced JSON carries the genesis
ASM predicate and the trusted mapping from predicates to compiled native spec IDs.
Distribute it with the network parameters and preserve existing mappings across
restarts. Upstream ASM validates the catalog structure, but does not authenticate
the mapping. These parameters are independent of checkpoint proving settings.

Compose mounts `configs/dev/asm-execution-params.json` read-only at
`/app/configs/asm-execution-params.json`. Its `AlwaysAccept`
predicate and spec 0 are development parameters; deployments must supply their
network's trusted catalog. The node loads the file before opening its database.

ASM v0.4.0 also changes `asm-params.json`: admin `signers` are P2WPKH
addresses on the anchor's Bitcoin network. The bridge address is named
`safe_harbor_address`, and its admin confirmation depth is named
`safe_harbor_address_update`. Datatool emits the new format; existing parameter
files must be converted separately.
