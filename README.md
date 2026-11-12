# Strata Chain

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache-blue.svg)](https://opensource.org/licenses/apache-2-0)
[![codecov](https://codecov.io/gh/alpenlabs/alpen/branch/main/graph/badge.svg?token=Q3ZYY44GN7)](https://codecov.io/gh/alpenlabs/strata)
[![ci](https://github.com/alpenlabs/alpen/actions/workflows/lint.yml/badge.svg?event=push)](https://github.com/alpenlabs/alpen/actions)

Strata is a blockchain that settles on Bitcoin and posts zero-knowledge proven state root checkpoints of its chain to Bitcoin.
This repository holds its Orchestration Layer (OL): the `strata` node, the detached signer
and the checkpoint prover. The Strata chain is complementary to the [`strata-bridge`](https://github.com/alpenlabs/strata-bridge).

## What is Strata?

Strata is built from two state machines.
Each one includes a state transition function that runs natively in a node and inside a zkVM for proofs.

- The **Anchor State Machine** (ASM, [asm](https://github.com/alpenlabs/asm)) reads Bitcoin
  and picks out the transactions meant for Strata: governance, checkpoints and the bridge.
- The **Orchestration Layer** (OL), in this repository, runs the chain:
  accounts, messages between them, epochs, deposits and withdrawals.
  At the end of each epoch it posts a checkpoint to Bitcoin
  with a validity proof and the data needed to rebuild its state.

Applications run on the OL as snark accounts.
An execution environment (EE) such as the EVM in [`alpen-ee`](https://github.com/alpenlabs/alpen-ee) is one:
it advances its account with proofs, and the OL checks each proof against the account's
predicate instead of re-running the EE.

## What is in this repository

- `strata`: the node. It runs as the sequencer, which builds blocks and posts checkpoints,
  or as a follower node that follows the checkpoints on Bitcoin.
- `strata-signer`: the detached signer that holds the sequencer's keys.
- `strata-datatool`, `strata-dbtool` and `strata-test-cli`: tools for generating parameters,
  inspecting the database and driving the functional tests.
- The OL state transition function and its types, and the SP1 checkpoint prover.
- Python functional tests that run the binaries against `bitcoind`.

## Goals

- **Bitcoin is the source of truth.** An OL block confirmed in a Bitcoin checkpoint
  can only be reorganized by reorganizing that Bitcoin block.
- **Provable.** The same state transition runs natively and inside a zkVM,
  so anyone can check a checkpoint from its proof instead of re-running it.
- **Rebuildable.** A follower node rebuilds the OL state from the data in the checkpoints on Bitcoin.
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
