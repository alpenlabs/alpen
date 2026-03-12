# Alpen Client Docker Deployment

Run a local Alpen network: bitcoind + strata (OL) + alpen-client sequencer + alpen-client fullnode.

## Architecture

```
bitcoind (regtest)
    |
    v
strata (OL sequencer)  <--- processes L1 blocks, manages OL state
    |
    v
alpen-client (EE sequencer)  <--- builds EE blocks, connected to strata via WebSocket
    |
    v
alpen-client-fullnode  <--- syncs from sequencer via P2P, read-only EE node
```

## Prerequisites

- Docker (with compose v2)
- Rust toolchain (to build `datatool`)
- Python 3 with `coincurve` (`pip install coincurve`)

## Quick Start

```bash
cd docker/

# 1. Build datatool
cargo build --release --bin strata-datatool

# 2. Generate keys and params
./init-alpen-client-keys.sh ../target/release/strata-datatool

# 3. Build docker images
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml build

# 4. Start the network
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml up -d

# 5. Check logs
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml logs -f
```

## Key Generation

The `init-alpen-client-keys.sh` script uses `datatool` to generate all keys and params.
This replaces the old approach of hardcoded JSON heredocs, ensuring params match the
actual serialization format expected by the binaries.

### What it generates

| File | Description |
|------|-------------|
| `configs/alpen-client/sequencer.key` | Sequencer master xpriv (BIP32 root) |
| `configs/alpen-client/operator.key` | Bridge operator master xpriv |
| `configs/alpen-client/sequencer-schnorr.hex` | Schnorr signing key (32 bytes hex) |
| `configs/alpen-client/seq-p2p.hex` | Sequencer P2P key (32 bytes hex) |
| `configs/alpen-client/fn-p2p.hex` | Fullnode P2P key (32 bytes hex) |
| `configs/alpen-client/jwt.hex` | Engine API JWT secret (32 bytes hex) |
| `configs/alpen-client/rollup-params.json` | Rollup parameters (magic bytes, genesis, operators) |
| `configs/alpen-client/ol-params.json` | OL genesis parameters (accounts, L1 view) |
| `configs/alpen-client/asm-params.json` | ASM parameters (subprotocols, bridge config) |
| `configs/alpen-client/genesis-l1-view.json` | L1 genesis view (placeholder, patched at runtime) |
| `configs/alpen-client/sequencer.toml` | Sequencer config (block time, epoch sealing) |
| `.env.alpen-client` | Environment variables for docker compose |

### Regeneration

The script is idempotent: existing files are preserved. To regenerate everything:

```bash
rm -rf configs/alpen-client/ .env.alpen-client
./init-alpen-client-keys.sh ../target/release/strata-datatool
```

### Signet

```bash
BITCOIN_NETWORK=signet ./init-alpen-client-keys.sh ../target/release/strata-datatool
```

## Fullnode Mode

To set up a standalone fullnode that syncs from an existing sequencer:

```bash
# On the fullnode machine, copy params from the sequencer's configs dir:
./init-alpen-client-keys.sh --fullnode ../target/release/strata-datatool \
    --params-dir /path/to/sequencer/configs/alpen-client

# This generates:
#   - configs/alpen-client/fn-p2p.hex  (new P2P key)
#   - configs/alpen-client/rollup-params.json  (copied from sequencer)
#   - configs/alpen-client/ol-params.json  (copied from sequencer)
#   - configs/alpen-client/asm-params.json  (copied from sequencer)
#   - .env.alpen-client-fullnode  (env file for fullnode compose)
```

## Services

### bitcoind

Regtest Bitcoin node with RPC and wallet support.

- RPC port: `${BITCOIND_RPC_PORT:-18443}`
- Health check: `bitcoin-cli getblockchaininfo`

### strata (OL)

Orchestration Layer sequencer. Processes L1 blocks, manages OL state, produces checkpoints.

- RPC port: `${STRATA_RPC_PORT:-8432}`
- Config: `configs/config.regtest.toml`
- Entrypoint patches genesis L1 height from the actual bitcoind tip at startup

### alpen-client (sequencer)

EE sequencer node. Builds execution blocks, publishes DA to L1.

- HTTP RPC: `${SEQ_HTTP_PORT:-8545}`
- WebSocket: `${SEQ_WS_PORT:-8546}`
- P2P: `${SEQ_P2P_PORT:-30303}`
- Chain spec: `${CHAIN_SPEC:-dev}` (chainId 2892, Cancun fork, dev account funded)
- Requires `SEQUENCER_PRIVATE_KEY` env var

### alpen-client-fullnode

Read-only EE node. Syncs from sequencer via P2P, forwards txns to sequencer.

- HTTP RPC: `${FN_HTTP_PORT:-9545}`
- WebSocket: `${FN_WS_PORT:-9546}`
- P2P: `${FN_P2P_PORT:-31303}`
- Bootnodes: auto-configured to connect to sequencer

## Verifying the Deployment

### Check service health

```bash
# All services running
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml ps

# Bitcoind
curl -s -u rpcuser:rpcpassword -d '{"jsonrpc":"1.0","method":"getblockchaininfo","params":[]}' \
    http://localhost:18443 | jq .result.blocks

# Strata (OL)
curl -s -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"strata_protocolVersion","params":[],"id":1}' \
    http://localhost:8432

# Alpen-client sequencer (EE)
curl -s -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
    http://localhost:8545

# Alpen-client fullnode (EE)
curl -s -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
    http://localhost:9545
```

### Check block production

```bash
# Watch EE block numbers increase (sequencer)
watch -n2 'curl -s -X POST -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"eth_blockNumber\",\"params\":[],\"id\":1}" \
    http://localhost:8545 | jq -r .result'
```

### Send an EE transaction (dev chain)

The `dev` chain spec pre-funds account `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` with 1M ETH.
Private key: `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80`

```bash
# Send a simple value transfer using cast (from foundry)
cast send \
    --rpc-url http://localhost:8545 \
    --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
    0x0000000000000000000000000000000000000001 \
    --value 1ether

# Check balance
cast balance --rpc-url http://localhost:8545 0x0000000000000000000000000000000000000001
```

### Check logs

```bash
# All services
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml logs -f

# Specific service
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml logs -f strata
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml logs -f alpen-client
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml logs -f alpen-client-fullnode
```

## Teardown

```bash
# Stop services
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml down

# Stop and remove volumes/data
docker compose --env-file .env.alpen-client -f docker-compose-alpen-client.yml down -v
rm -rf data/
```

## Environment Variables

All configurable via environment or the generated `.env.alpen-client` file:

| Variable | Default | Description |
|----------|---------|-------------|
| `BITCOIN_NETWORK` | `regtest` | Bitcoin network (regtest, signet) |
| `CHAIN_SPEC` | `dev` | EVM chain spec (dev, devnet, testnet) |
| `EE_DA_MAGIC_BYTES` | `ALPT` | EE DA transaction magic bytes |
| `ALPEN_EE_BLOCK_TIME_MS` | `5000` | EE block time in milliseconds |
| `OL_BLOCK_TIME_MS` | `5000` | OL block time in milliseconds (in sequencer.toml) |
| `L1_REORG_SAFE_DEPTH` | `4` | L1 finality depth |
| `BATCH_SEALING_BLOCK_COUNT` | `5` | EE blocks per batch |
| `RUST_LOG` | `info` | Log level (trace, debug, info, warn, error) |

## Troubleshooting

**"datatool not found or not executable"**: Build it first with `cargo build --release --bin strata-datatool`.

**"no python with coincurve found"**: Install with `pip install coincurve`.

**Strata fails with "missing sequencer config"**: Ensure `sequencer.toml` exists in `configs/alpen-client/`. Re-run the init script.

**L1 genesis height mismatch**: The strata entrypoint patches genesis height from the actual bitcoind tip. If bitcoind data was reset but params weren't, regenerate: `rm -rf configs/alpen-client/ .env.alpen-client data/` and re-run.

**Fullnode not syncing**: Check that the sequencer P2P port (30303) is reachable and `SEQ_P2P_PUBKEY` in the env file matches the actual key.
