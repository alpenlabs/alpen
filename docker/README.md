# Docker

Docker setups for the alpen-client (EE) node. The strata (OL) node images
live in the strata repo; local full-stack composes will return once the
post-split params/keys flow is in place (see the repo-split notes).

## Compose files

| Compose | Purpose |
|---|---|
| `compose-signet.yml` | Local signet `bitcoind` miner or fullnode |
| `docker-compose-eest.yml` | Ethereum execution spec test environment |
| `docker-compose-p2p-test.yml` | Minimal EE P2P/gossip test |

## Images

| Directory | Image |
|---|---|
| `alpen-client/` | EE node (`Dockerfile` for CI/registry builds, `Dockerfile.local` for local compose builds) |
| `bitcoind/` | Regtest bitcoind used by the test composes |

See `simple-doc.md` for alpen-client setup and operations.
