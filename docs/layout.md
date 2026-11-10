# Repository layout

This page lists each crate and what it does. Package names are in parentheses; most carry a
`strata-` prefix. Each crate's `lib.rs` doc comment has more. The workspace builds
`bin/strata`, `bin/strata-dbtool`, `bin/strata-signer`, `bin/datatool`, and
`bin/strata-test-cli` by default; check `default-members` in the root `Cargo.toml` before
assuming another crate builds.

## OL state transition

These crates make up the OL STF. The checkpoint guest compiles them, so a change here can
move the checkpoint verifying key (see [ol-specs.md](ol-specs.md#proofs-per-spec)).

- [`crates/ol/stf`](../crates/ol/stf) (`strata-ol-stf`): the versioned STF entry point. It
  dispatches each operation on `OLSpecId`.
- [`crates/ol/stf-v1`](../crates/ol/stf-v1) (`strata-ol-stf-v1`): the V1 rules: block, epoch,
  manifest, and transaction processing.
- [`crates/ol/state-types`](../crates/ol/state-types) (`strata-ol-state-types`): state accessor
  traits, ledger entry types, `OLSpecId`, and the fixed `OLRootState`.
- [`crates/ol/state-types-v1`](../crates/ol/state-types-v1) (`strata-ol-state-types-v1`):
  `OLStateV1` and its parts: global, epochal, ledger, and snark account state.
- [`crates/ol/state-container`](../crates/ol/state-container) (`strata-ol-state-container`):
  `OLStateContainer`, `OLStateSeries`, the spec-to-layout map, and the container's serde form.
- [`crates/ol/state-support-types`](../crates/ol/state-support-types)
  (`strata-ol-state-support-types`): state access layers: the in-memory base layer, write
  batches, write tracking, and the indexer.
- [`crates/ol/chain-types-v1`](../crates/ol/chain-types-v1) (`strata-ol-chain-types-v1`): OL
  block types. Re-exports the OL log types from strata-common.
- [`crates/ol/tx-types-v1`](../crates/ol/tx-types-v1) (`strata-ol-tx-types-v1`): OL
  transactions, their payloads, and transaction proofs.
- [`crates/ol/msg-types`](../crates/ol/msg-types) (`strata-ol-msg-types`): deposit and
  withdrawal message types.
- [`crates/ol/da-common`](../crates/ol/da-common) (`strata-ol-da-common`): DA traits and
  encoding helpers shared by DA payload versions.
- [`crates/ol/da-types-v1`](../crates/ol/da-types-v1) (`strata-ol-da-types-v1`): the V1 DA
  payload, its scheme, and the checkpoint transaction extractor.
- [`crates/ol/params`](../crates/ol/params) (`strata-ol-params`): `OLGenesisParams`,
  `OLRuntimeParams`, and `OLParams`.
- [`crates/da-framework`](../crates/da-framework) (`strata-da-framework`): DA diff primitives:
  `DaRegister`, `DaCounter`, `DaLinacc`, `DaQueue`, and compound types.

## Accounts and bridge

- [`crates/acct-types`](../crates/acct-types) (`strata-acct-types`): account IDs, messages,
  and effects.
- [`crates/snark-acct-types`](../crates/snark-acct-types) (`strata-snark-acct-types`): snark
  account state, updates, and the proof interface.
- [`crates/snark-acct-sys`](../crates/snark-acct-sys) (`strata-snark-acct-sys`): snark account
  update handling and verification.
- [`crates/snark-acct-runtime`](../crates/snark-acct-runtime) (`strata-snark-acct-runtime`): a
  general-purpose snark account runtime library.
- [`crates/bridge-types`](../crates/bridge-types) (`strata-ol-bridge-types`): bridge operation
  and message types shared with the EE.
- [`crates/bridge-params`](../crates/bridge-params) (`strata-bridge-params`): bridge
  denomination and withdrawal-cap parameters.

## OL node services

- [`crates/chain-worker`](../crates/chain-worker) (`strata-chain-worker`): executes and
  persists OL blocks and epochs.
- [`crates/consensus-logic`](../crates/consensus-logic) (`strata-consensus-logic`): fork
  choice, checkpoint sync, the unfinalized block tracker, MMR reconciliation, and the ASM and
  CSM worker contexts.
- [`crates/csm-worker`](../crates/csm-worker) (`strata-csm-worker`): follows the ASM worker and
  turns checkpoint logs into client state.
- [`crates/ol/canonical-chain-provider`](../crates/ol/canonical-chain-provider)
  (`strata-ol-canonical-chain-provider`): read-only canonical block and epoch queries, from
  fork choice or from finalized checkpoints.
- [`crates/ol/spec-selection`](../crates/ol/spec-selection) (`strata-ol-spec-selection`):
  selects the next epoch's spec from stored epoch summaries and manifests. Not in the guest.
- [`crates/ol/state-provider`](../crates/ol/state-provider) (`strata-ol-state-provider`):
  traits and implementations for reading OL state.
- [`crates/ol/mmr-index`](../crates/ol/mmr-index) (`strata-ol-mmr-index`): compares and
  reconciles OL-owned MMRs with the database index.
- [`crates/ol/genesis`](../crates/ol/genesis) (`strata-ol-genesis`): builds the OL genesis
  state and block.
- [`crates/ol/checkpoint`](../crates/ol/checkpoint) (`strata-ol-checkpoint`): the checkpoint
  builder and the tracker for checkpoints the ASM rejected.
- [`crates/ol/rpc/api`](../crates/ol/rpc/api) and [`types`](../crates/ol/rpc/types)
  (`strata-ol-rpc-api`, `strata-ol-rpc-types`): the OL JSON-RPC traits and wire types.

## Sequencer

- [`crates/ol/mempool`](../crates/ol/mempool) (`strata-ol-mempool`): pending OL transactions.
- [`crates/ol/block-assembly`](../crates/ol/block-assembly) (`strata-ol-block-assembly`):
  builds block templates and decides when to seal an epoch.
- [`crates/ol/log-budget`](../crates/ol/log-budget) (`strata-ol-log-budget`): log accounting
  and per-transaction log budget checks for admission and assembly.
- [`crates/ol/sequencer`](../crates/ol/sequencer) (`strata-ol-sequencer`): signing duties for
  blocks, checkpoints, and envelope reveals.
- [`crates/strata-signer`](../crates/strata-signer) (`strata-signer`): the detached signer
  service that `bin/strata-signer` runs.
- [`crates/key-derivation`](../crates/key-derivation) (`strata-key-derivation`): sequencer key
  derivation.

## Bitcoin I/O

- [`crates/btcio`](../crates/btcio) (`strata-btcio`): the L1 reader, the envelope writer and
  bundler, fee bumping, and the broadcaster.

Bitcoin primitives, header verification, and SPS-50 parsing come from the pinned strata-common
and asm crates.

## Storage

- [`crates/db/types`](../crates/db/types) (`strata-db-types`): database traits and record
  types.
- [`crates/db/store-sled`](../crates/db/store-sled) (`strata-db-store-sled`): the sled
  implementation.
- [`crates/storage`](../crates/storage) (`strata-storage`): cached managers and `NodeStorage`.
- [`crates/storage-common`](../crates/storage-common) (`strata-storage-common`): shared
  storage helpers.
- [`crates/db/tests`](../crates/db/tests) (`strata-db-tests`): macros that run one test suite
  against every database implementation.

## Proofs

- [`crates/proof-impl/checkpoint`](../crates/proof-impl/checkpoint)
  (`strata-proofimpl-checkpoint`): the checkpoint proof statement.
- [`crates/proof-impl/predicate-keys`](../crates/proof-impl/predicate-keys)
  (`strata-proofimpl-predicate-keys`): predicate keys for proof programs.
- [`crates/ol/checkpoint-artifacts`](../crates/ol/checkpoint-artifacts)
  (`strata-ol-checkpoint-artifacts`): loads checkpoint program bundles and looks them up by
  spec.
- [`crates/prover-core`](../crates/prover-core) (`strata-prover-core`): a proving engine for
  one proof type, native or remote.
- [`crates/paas`](../crates/paas) (`strata-paas`): Prover-as-a-Service around `prover-core`.
- [`crates/zkvm/hosts`](../crates/zkvm/hosts) (`strata-zkvm-hosts`): zkVM hosts. SP1 is behind
  the `sp1` feature.
- [`provers/sp1`](../provers/sp1) (`strata-sp1-guest-builder`): builds the SP1 checkpoint
  guest into `generated/`. The guest, `guest-checkpoint/`, is its own workspace with its own
  `Cargo.lock`.

## Shared

- [`crates/primitives`](../crates/primitives) (`strata-primitives`): widely used data types.
- [`crates/checkpoint-types`](../crates/checkpoint-types) (`strata-checkpoint-types`):
  checkpoint, batch, terminal header, and prover task types.
- [`crates/csm-types`](../crates/csm-types) (`strata-csm-types`): client state types.
- [`crates/config`](../crates/config) (`strata-config`): the node's TOML configuration.
- [`crates/common`](../crates/common) (`strata-common`): service helpers: health checks,
  the bail and worker-pause hooks the crash tests use, and a WebSocket client. Not the
  strata-common repository.
- [`crates/status`](../crates/status) (`strata-status`): the status channel services publish
  sync state on.
- [`crates/node-context`](../crates/node-context) (`strata-node-context`): the context passed to
  node services at startup.
- [`crates/cli-common`](../crates/cli-common) (`strata-cli-common`): shared CLI error types.
- [`crates/rpc/open-rpc`](../crates/rpc/open-rpc), [`open-rpc-macros`](../crates/rpc/open-rpc-macros),
  and [`open-rpc-spec`](../crates/rpc/open-rpc-spec): OpenRPC types, derive macros, and the OL
  OpenRPC document. `cargo run -p strata-rpc-openrpc-spec` prints the document.

## Binaries

- [`bin/strata`](../bin/strata) (`strata`): the node. Sequencer, full node, RPC, and the
  in-process prover behind the `prover` and `sp1` features.
- [`bin/strata-signer`](../bin/strata-signer) (`strata-signer-bin`, binary `strata-signer`):
  the detached sequencer signer.
- [`bin/strata-dbtool`](../bin/strata-dbtool/README.md) (`strata-dbtool`): offline database
  inspection and repair.
- [`bin/datatool`](../bin/datatool/README.md) (`strata-datatool`): generates keys and the ASM
  and OL parameter files.
- [`bin/strata-test-cli`](../bin/strata-test-cli) (`strata-test-cli`): builds bridge and EE
  transactions for the functional tests, and includes mock EE helpers.
- [`bin/prover-perf`](../bin/prover-perf/README.md) (`strata-provers-perf`): proof performance
  reports.

The downstream [alpen-ee](https://github.com/alpenlabs/alpen-ee) repository builds `strata`,
`strata-signer`, `strata-datatool`, and `strata-test-cli` from a pinned revision for its own
functional tests, so their CLIs are public interfaces.

## Tests

- [`crates/test-utils/test-utils`](../crates/test-utils/test-utils) (`strata-test-utils`):
  `Arbitrary`-based generators.
- [`crates/test-utils/l2`](../crates/test-utils/l2) (`strata-test-utils-l2`): OL, ASM, and
  checkpoint fixtures.
- [`functional-tests`](../functional-tests/README.md): Python tests that run the binaries
  against `bitcoind`, grouped under `tests/` by component.
- [`benches`](../benches) (`alpen-benchmarks`): Criterion benchmarks for the database.

## Other

- [`contrib`](../contrib): style and TODO check scripts used by `just lint`, and dev helpers.
- [`docker`](../docker/README.md): images and compose files for a local signet and sequencer
  stack.
- [`.github/workflows`](../.github/workflows): CI, the checkpoint VK guard, and the SP1
  artifact publisher.

Shared protocol types come from [strata-common](https://github.com/alpenlabs/strata-common)
and the ASM from [asm](https://github.com/alpenlabs/asm). Both are pinned by tag in the root
`Cargo.toml`.
