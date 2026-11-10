# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Overview

Alpen is an EVM-compatible Bitcoin layer 2 built from three state machines: the Anchor State
Machine (ASM) reads Bitcoin, the Orchestration Layer (OL) runs the L2 chain, and the
Execution Environment (EE) runs the EVM. This repository holds the OL: the `strata` node, the
detached signer, the OL state transition function (STF), its tools, and the SP1 checkpoint
prover.

The ASM lives in [alpenlabs/asm](https://github.com/alpenlabs/asm) and shared protocol
primitives in [alpenlabs/strata-common](https://github.com/alpenlabs/strata-common); both are
pinned by tag in the root `Cargo.toml`. The EE lives downstream in
[alpenlabs/alpen-ee](https://github.com/alpenlabs/alpen-ee), which consumes this repository's
crates and builds `strata`, `strata-signer`, `strata-datatool`, and `strata-test-cli` from a
pinned revision. Changes to shared types, RPC, or those binaries' CLIs are breaking for it.
This repository has no EVM, Reth, or Revm code.

Contributor docs are in [`docs/`](docs/README.md). Read
[`docs/architecture.md`](docs/architecture.md) before changing the STF or the node services,
and [`docs/ol-specs.md`](docs/ol-specs.md) before changing consensus rules.

## Commands

```bash
just fmt-ws               # format Rust
just lint                 # all format and lint checks
just lint-check-ws        # clippy alone, warnings denied
just test-unit            # cargo nextest over the workspace
just test-doc             # doctests
just pr-lite              # lints, rustdocs, doctests, and unit tests
cargo nextest run --locked -p <crate>              # one crate
cd functional-tests && ./run_tests.sh -t <test>    # one functional test
cd functional-tests && ./run_tests.sh -g <group>   # one functional test group
```

`just lint` needs `taplo`, `codespell`, `shellcheck`, and `uv`. Builds with `--all-features`,
which `just lint-check-ws` and CI use, need `protoc` for the SP1 crates. Functional tests need
`bitcoind` and `uv`; CI uses Bitcoin Core 30.2. `nix develop` provides all of these; see
[`CONTRIBUTING.md`](CONTRIBUTING.md).

The functional test runner builds `strata`, `strata-signer`, `strata-datatool`,
`strata-test-cli`, and `strata-dbtool` with `-F sequencer -F debug-utils -F prover`. Failed
runs leave logs under `functional-tests/_dd/`, in a directory named after the run.

## Architecture

See [`docs/layout.md`](docs/layout.md) for every crate.

- `crates/ol/stf`, `crates/ol/stf-v1`: the STF. All drivers call `ol/stf` with the epoch's
  `OLSpecId`; only tests and benchmark fixtures call `ol/stf-v1` directly.
- `crates/ol/*-types*`, `crates/acct-types`, `crates/snark-acct-*`, `crates/da-framework`:
  consensus types and DA. Concrete types live in `-v1` crates and carry a `V1` suffix
  (`OLBlockV1`, `OLStateV1`); unsuffixed crates hold version-independent traits.
- `crates/chain-worker`, `crates/consensus-logic`, `crates/csm-worker`, `crates/ol/checkpoint`:
  node services that execute blocks, choose the tip, follow L1 checkpoints, and build
  checkpoints.
- `crates/ol/mempool`, `crates/ol/block-assembly`, `crates/ol/sequencer`,
  `crates/strata-signer`: sequencing and signing duties.
- `crates/btcio`: the L1 reader, envelope writer, and broadcaster.
- `crates/db/*`, `crates/storage`: the sled database and its cached managers.
- `crates/proof-impl/checkpoint`, `crates/ol/checkpoint-artifacts`, `provers/sp1`: the
  checkpoint proof, its per-spec artifacts, and the SP1 guest.
- `bin/strata`: wires `bitcoind`, the ASM worker, the services, storage, the prover, and RPC.
- `functional-tests/`: Python tests against the binaries, grouped by component.

## Testing

- Add tests for behavior changes and regressions. Test public behavior and protocol
  invariants, not private details.
- Keep unit tests free of external processes. Anything that needs a running node, `bitcoind`,
  or the signer goes in `functional-tests/`.
- Reuse shared fixtures and wait helpers: `crates/test-utils/*`, the `crates/db/tests` macros,
  the `strata-ol-stf-v1` test helpers, and `functional-tests/common` (`wait_until` and the
  service `wait_for_*` methods). Add missing ones there instead of copying them into a test.
- Don't re-test what an upstream library already guarantees.
- Pin consensus behavior with fixed values. The golden V1 test
  (`crates/ol/checkpoint/src/tests/golden_v1.rs`) and the DA payload fixtures in
  `crates/ol/da-types-v1` fail on purpose when execution or an encoding moves. Find out what
  changed before you record new values; never re-record just to make a test pass.
- Exercise the real assembly or encoding path together with its verification or decoding
  path.
- Make tests deterministic. Wait on conditions, not sleeps. Prefer `assert_eq!` over
  `assert!` when comparing values. Name tests for the behavior, for example
  `test_deposit_with_invalid_amount_fails`.
- Run the affected crate first, then widen.

## Commit and PR Style

- Use Conventional Commits with a scope, for example `fix(btcio): ...`. Add `!` for a breaking
  change, for example `feat(ol)!: ...`.
- Keep each commit to one logical change that builds and passes tests on its own. Keep history
  linear: rebase instead of merging, and push with `--force-with-lease`.
- Follow [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md), including the
  AI-use disclosure. Use the Conventional Commit format for the PR title.
- Keep PRs focused. Put unrelated refactors, migrations, and cleanup in their own PR.
- Run `just fmt-ws`, `just lint`, and the tests for what you changed. Don't add new warnings.
  `just lint-check-ws` fails on clippy warnings, as CI does. CI also fails on `cargo doc`
  warnings, so run `just rustdocs` if you changed doc comments. Say in the PR what you could
  not run.

## Notes

- **Determinism**: code the checkpoint guest compiles must not depend on wall-clock time,
  environment variables, I/O, unseeded randomness, or `HashMap` iteration order. That is the
  STF, the state, transaction, block, account, and DA types, and the proof statement.
- **Consensus changes**: changing an encoded field, its order, a hash input, a validity rule,
  or a capacity is a consensus change. Mark it breaking with `!`. Once a chain runs the old
  rules, the change ships as a new OL spec; see [`docs/ol-specs.md`](docs/ol-specs.md). The
  state root is the root of `OLRootState`, not of `OLStateV1`, from V1 on.
- **Checkpoint verifying key**: a change to any crate the guest compiles, to the toolchain,
  or to `Cargo.lock` can move the SP1 verifying key, which is consensus. The checkpoint VK
  guard workflow builds the guest at the PR base and the merge commit and flags a moved key.
  When you add a local dependency to the guest, add its path to that workflow's filter. The
  embedded `OLRuntimeParams` are part of the key too.
- **Guest**: `provers/sp1/guest-checkpoint` is its own workspace with its own `Cargo.lock`.
  Update that lockfile when you change a dependency the guest uses. Building the guest needs
  `BUILD_ELF=1` and `CHECKPOINT_RUNTIME_PARAMS_PATH`; add `--features docker-build` for a
  reproducible key. Output in `provers/sp1/generated/` is not tracked and survives
  `cargo clean`. See [`provers/sp1/README.md`](provers/sp1/README.md).
- **SSZ schemas**: `crates/acct-types`, `crates/snark-acct-types`, `crates/ol/state-types`,
  `crates/ol/state-types-v1`, `crates/ol/chain-types-v1`, and `crates/ol/tx-types-v1` generate
  their SSZ types in `build.rs` from the `.ssz` files in their `ssz/` directory. Edit the
  schema, not the generated code.
- **Parameters**: ASM parameters, OL parameters, and node config are separate files with
  separate owners. Don't duplicate protocol parameters in node config. Changing
  `OLRuntimeParams` changes the checkpoint verifying key.
- **TODOs**: every new `TODO` or `FIXME` needs a ticket, for example `TODO(STR-1234): ...`.
  `just lint-check-todos` runs the same check as CI.
- **Dependencies**: declare versions in the root `Cargo.toml` and use `workspace = true`.
  `.cargo/config.toml` refuses crates.io versions less than 14 days old. Borsh has been
  removed from the workspace; don't reintroduce it.
- **Fixtures**: the root `.gitignore` ignores every `data/` directory, so files under one are
  silently left out of commits. Put fixtures under `testdata/`.

## Code Style

### Rust

- Parse, don't validate: encode invariants in types so invalid states can't be built. Use
  typed IDs and domain values inside, and convert to strings only at boundaries.
- Avoid heap allocation in pure library crates. Prefer the stack and skip `Arc` with no
  sharing need.
- Borrow to inspect, take ownership when you need it, and use `&mut` for in-place updates.
  Avoid needless clones, temporary collections, and repeated encoding buffers.
- Implement `Default` only when the type has a real domain default; use fixtures or
  generators for arbitrary test values. Add `const` only when compile-time use is part of the
  API.

### Design and API Boundaries

- Give each crate one focused job. Keep protocol and STF logic independent of RPC,
  persistence, orchestration, and runtime policy.
- Take dependencies through narrow context traits. Prefer the existing state accessor traits
  over concrete state types. Keep concrete databases and services in the integration layer.
- Keep binaries to loading config, opening resources, setting up observability, and
  launching library services. Put RPC, sync, and processing logic in library crates.
- Don't expose `pub` fields on nontrivial domain structs. Provide constructors, accessors,
  borrowed views, and operations that keep invariants. Plain data carriers with no
  invariants may have public fields.
- Keep constructors to field assembly and basic checks. Give substantial setup or I/O its own
  named function.
- Reuse the owning implementation of protocol algorithms, validation, assembly, codecs, and
  test helpers. Don't duplicate them at call sites. Don't take independent arguments that can
  form an incoherent state.
- Derive settings from their authoritative source instead of asking users to configure
  inferable values. Low-level crates own their config types and must not depend on the
  top-level node config.

### Serialization

| Context | Format |
| --- | --- |
| Protocol data structures | SSZ (`ssz`, `ssz_derive`, `tree_hash` from `alpenlabs/ssz-gen`), custom `.ssz` schemas |
| On-chain envelope payloads | `strata-codec` |
| OL state snapshots and the proof's start state | serde over the root fields and SSZ chainstate bytes (`strata-ol-state-container`) |
| Private proof interfaces and sled values | `rkyv` |
| Other persistent data | CBOR (`ciborium`) |
| Human-readable config | JSON or TOML through `serde` |

- Use the codec the boundary already uses. Don't add a wire or storage format without a
  compatibility and migration plan.
- Keep domain types separate from wire or storage types when their fields or invariants
  differ. Validate at the conversion.

### Naming and Documentation

- Directories `kebab-case`; files, modules, functions, variables, and serde fields
  `snake_case`; types and traits `UpperCamelCase`; constants `SCREAMING_SNAKE_CASE`.
- Use precise verbs for functions that do work. Use bare nouns for cheap accessors, `as_` for
  cheap borrowed views, `to_` for allocating conversions, and `with_` for builders.
- Import with `use` at the top of the file. CI denies absolute paths through
  `clippy::absolute_paths`.
- Write docs in active voice, third-person indicative. Give public items a one-sentence
  summary. Use intra-doc links such as ``[`SomeType`]``. Document invariants, ordering,
  preconditions, panics, and design reasons.
- Comments explain why, not what.

### Error Handling

| Context | Approach |
| --- | --- |
| Invalid input or recoverable failure | `Result` with structured variants |
| Expected absence | `Option`, not a sentinel or default |
| Broken internal invariant | `assert!`, `unwrap()`, or `expect("the invariant")` |
| Library errors | an error `enum` or `struct` with `thiserror` |
| Application boundary errors | `anyhow` with context |

- Panics mean bugs or broken invariants, never normal runtime errors. Document them in a
  `# Panics` section on public APIs.
- Keep error variants and source chains distinct across layers. Don't collapse failures into
  strings or catch-all variants.
- Error messages describe the failure without an `error` prefix.

### Async and Concurrency

- Never block an async executor thread. Use an async API or the runtime's blocking-task
  facility.
- Don't hold a lock guard across `.await`.
- Workers own their state. Expose commands and status through a handle.

### Observability

- Use structured `tracing` fields, not interpolated strings, and use the shorthand when the
  field name matches the variable:

  ```rust
  info!(%block_id, height, "processing block");
  ```

- Don't add ad hoc `component` fields or repeat context the module or span already gives.
- `error!` for unrecoverable failures, `warn!` for unexpected conditions someone should act
  on, `info!` for significant events, `debug!` and `trace!` for detail.
- Expected lag, graceful shutdown, irrelevant traffic, and rejected untrusted input should
  not warn.
- Keep pure STF code free of operational logs when the caller can report with better
  context.
- Pick metrics by shape: `Counter` for monotonic counts, `UpDownCounter` for values that rise
  and fall, `Gauge` for point-in-time values, `Histogram` for distributions.
