# Alpen

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache-blue.svg)](https://opensource.org/licenses/apache-2-0)
[![codecov](https://codecov.io/gh/alpenlabs/alpen/branch/main/graph/badge.svg?token=Q3ZYY44GN7)](https://codecov.io/gh/alpenlabs/strata)
[![ci](https://github.com/alpenlabs/alpen/actions/workflows/lint.yml/badge.svg?event=push)](https://github.com/alpenlabs/alpen/actions)
[![docs](https://img.shields.io/badge/docs-strata-orange)](https://docs.stratabtc.org)

<p align="center">
  <img src="https://raw.githubusercontent.com/alpenlabs/.github/76e8490215c5c1e21d98ad28380bdd88b1bf6e4d/profile/images/logo.png" alt="Alpen Labs Logo" width="21%">
</p>

The Orchestration Layer node for Alpen, an EVM-compatible Bitcoin layer 2.

## What is Alpen?

[**Alpen**](https://alpenlabs.io) gives developers the freedom to program nearly
any locking conditions for BTC imaginable,
limited only by the Alpen block size and gas limits.
This enables developers to create new kinds of applications for BTC
with features such as:

- **New signature types**, "provide a valid `P-256` signature to authorize a transfer"

- **Vaults**, "transfers must wait `N` days after being initiated to be effectuated,
  and can be cancelled in the mean time"

- **Subscriptions**, "address `0x123...9a` can withdraw up to `v` BTC
  per month from this account"

- **Strong privacy**, "transaction details are end-to-end encrypted
  and verified using a zero-knowledge proof"

- **Economically-secured zero-confirmation payments**,
  "if a double-spend from this sender is reported,
  the reporter gets to claim the sender's full wallet balance"

- **Financial transactions**,
  "if enough BTC is locked as collateral to maintain up
  to `X%` loan-to-value ratio,
  then up to `Y` of this other asset can be borrowed"

... and many more possibilities.

Alpen is built from three state machines.
The Anchor State Machine ([asm](https://github.com/alpenlabs/asm)) reads Bitcoin
and picks out the transactions meant for Alpen.
The Orchestration Layer (OL), in this repository, runs the L2 chain:
accounts, messages between them, deposits and withdrawals, and epochs.
At the end of each epoch it posts a checkpoint to Bitcoin
with a validity proof and the data needed to rebuild its state.
The Execution Environment ([alpen-ee](https://github.com/alpenlabs/alpen-ee))
runs the EVM and appears to the OL as one account that it advances with proofs.
If you can deploy a smart contract to Ethereum,
you can deploy it to Alpen with no changes.

This repository contains the `strata` node, which runs as the sequencer or as a full node
that follows checkpoints on Bitcoin, the detached `strata-signer`,
the tools around them, and the SP1 checkpoint prover.

To learn more, check our [documentation](https://docs.alpenlabs.io).

## Goals

- **Bitcoin is the source of truth.** An OL block confirmed in a Bitcoin checkpoint
  can only be reorganized by reorganizing that Bitcoin block.
- **Provable.** The same state transition runs natively and inside a zkVM,
  so anyone can check a checkpoint from its proof instead of re-running it.
- **Rebuildable.** A full node rebuilds the OL state from the data in the checkpoints on Bitcoin.
- **Upgradable.** New rules ship as a new spec that the chain switches to at an epoch boundary,
  after governance enacts it on Bitcoin.

## Contributing

Contributions are generally welcome.
If you intend to make larger changes please discuss them in an issue
before opening a PR to avoid duplicate work and architectural mismatches.

[`CONTRIBUTING.md`](CONTRIBUTING.md) lists the tools you need and the commands to lint and test.
For how the code fits together, see [`docs/`](docs/README.md):
start with [`docs/architecture.md`](docs/architecture.md),
then [`docs/layout.md`](docs/layout.md) for what each crate does.

## License

This work is dual-licensed under MIT and Apache 2.0.
You can choose between one of them if you use this work.
