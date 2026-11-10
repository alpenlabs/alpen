# OL specs

An OL spec is one version of the OL STF's rules, named by an `OLSpecId`
([`crates/ol/state-types/src/spec_id.rs`](../crates/ol/state-types/src/spec_id.rs)). Every epoch
runs under exactly one spec. A live chain changes rules only by moving to the next spec at an
epoch boundary. See [architecture.md](architecture.md) for where the STF fits in the node.

| Spec | Rules |
| --- | --- |
| V1 | The rules new networks start with |
| V2 | V1's rules with tighter capacities (see [Capacities](#capacities)) |

`OLGenesisParams` names the network's genesis spec as a required `genesis.spec`. New networks
use `1`; `2` starts a network under V2.

## Dispatch

[`crates/ol/stf`](../crates/ol/stf) is the one entry point to the STF. Each spec has a rule set
that implements the internal `OLStfSpec` trait (`v1.rs`, `v2.rs`), and the
`dispatch_spec!` macro in [`spec.rs`](../crates/ol/stf/src/spec.rs) maps each `OLSpecId` to its
rule set. The compiler makes every rule set provide every operation. Sequencer block assembly
composes the `sequencer` phases itself, so a rule set that changes their order must also
update [`crates/ol/block-assembly`](../crates/ol/block-assembly).

The state root records the spec, so every spec needs its own rule set even if it
changes no rules. V2 runs the V1 code.

## Spec selection

The caller picks the spec from the epoch it executes. A block runs under its header's epoch.
An epoch's terminal block, including the drain into the next epoch, runs under the spec of the
epoch it ends. Genesis runs under the genesis spec.

Each later epoch runs under the spec `select_next_epoch_spec` picks from the epoch before it:
that epoch's spec, or its `successor()` if that epoch processed a checkpoint predicate
enactment. The admin subprotocol enacts a new checkpoint predicate on L1, so every node reads
the switch from the manifests it already stores. No rule set stages or promotes a spec.
[`crates/ol/spec-selection`](../crates/ol/spec-selection) finds the L1 range the rule reads, for
every node-side driver. The checkpoint guest does not compile it, so changing it leaves the
verifying key alone.

A node must halt when an enactment needs a successor its binary does not know.

## State root

The OL state root is not the root of `OLStateV1`. Block headers, checkpoint terminal headers,
and genesis commit to the hash tree root of `OLRootState`
([`crates/ol/state-types/ssz/root.ssz`](../crates/ol/state-types/ssz/root.ssz)):

- `cur_spec_version`: the spec that produced the chainstate. It picks the layout.
- `staged_spec_version`: always equal to `cur_spec_version`, since no rules stage a spec.
- `chainstate_root`: the root of the chainstate in its layout, `OLStateV1` today.

`OLRootState::compute_state_root` computes it.

The first block of the first V2 epoch after a V1 state sets `cur = staged = V2` in
epoch-initial processing, so every driver, DA replay included, reaches the same root.

Genesis construction sets the initial versions (`new_genesis`). After that only the rules
change them, through `IStateAccessorMut::set_spec_versions` and the `WriteBatch` version slot.

### Layouts and the state container

`OLStateLayout::for_spec`
([`crates/ol/state-container/src/layout.rs`](../crates/ol/state-container/src/layout.rs)) maps
`cur_spec_version` to a chainstate layout. V1 and V2 both use `OLStateV1`. Spec and layout
numbers are never compared: several specs can share a layout.

`OLStateContainer` ([`crates/ol/state-container`](../crates/ol/state-container)) pairs the root
with an `OLStateSeries` chainstate. Storage, the chain worker, and the checkpoint proof input
carry it. It has no wire format of its own: serde writes the root's fields and the
chainstate's SSZ bytes (CBOR in sled, the zkVM's serde input in the proof), and decoding checks the
chainstate against the root. Execution uses `MemoryStateBaseLayer<OLStateV1>`, built with
`from_container` and turned back with `into_container`.

## Capacities

The V1 types bound transaction effects and block logs at 65,536 entries. Those bounds fix V1
txids and encodings, so they cannot shrink. V2 adds tighter capacities as validity
rules instead: 255 transfers and 255 messages per transaction, and 4,096 logs per block.
`spec_limits` ([`crates/ol/stf/src/limits.rs`](../crates/ol/stf/src/limits.rs)) returns a
spec's capacities for the drivers that build blocks or admit transactions.

## Proofs per spec

The SP1 guest in [`provers/sp1`](../provers/sp1/README.md) proves one spec, fixed by
`PROVED_SPEC` in `guest-checkpoint/src/spec.rs`. It proves V2 today. V1 epochs keep using the
bundle published from `v0.4.0-rc.1`. V2 proving accepts
terminal V1 states with `cur = staged = V1` as well as V2 start states. ASM key handover and
the verified-tip checks limit that V1 exception to the first accepted V2 epoch.

The node loads one artifact bundle per spec through
[`crates/ol/checkpoint-artifacts`](../crates/ol/checkpoint-artifacts/README.md), and picks the
bundle from the spec in each epoch's terminal state. It never uses the ASM's active predicate
for that.

The verifying key is consensus. The
[checkpoint VK guard](../.github/workflows/checkpoint-vk-guard.yml) builds the guest at the PR
base and at the merge commit and fails when the keys differ. Its path filter lists every
crate the guest compiles. The guard is advisory and not a required check.

## Adding a spec

A consensus change to a chain that already runs the current rules ships as a new spec:

1. Add the variant to `OLSpecId` and extend `successor()`.
2. Add its rule set in `crates/ol/stf` and its arm in `dispatch_spec!`. Set its capacities in
   `limits.rs`.
3. Map it to a layout in `OLStateLayout::for_spec`. A new chainstate layout also needs a new
   `OLStateSeries` variant.
4. Decide how its first epoch takes over the previous spec's state, as V1 and V2 do.
5. Move `PROVED_SPEC` to the new spec, publish its bundle, and keep the previous spec's bundle
   frozen. Add the new spec to the native registry in `crates/ol/checkpoint-artifacts`.
6. Cover the handover with STF tests, and keep the golden V1 test in
   [`crates/ol/checkpoint/src/tests/golden_v1.rs`](../crates/ol/checkpoint/src/tests/golden_v1.rs)
   passing unchanged.
