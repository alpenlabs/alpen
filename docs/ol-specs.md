# OL specs

An OL spec is one version of the OL STF's rules, named by an `OLSpecId`
([`crates/ol/state-types/src/spec_id.rs`](../crates/ol/state-types/src/spec_id.rs)). Every epoch
runs under exactly one spec. A live chain changes rules only by moving to the next spec at an
epoch boundary. See [architecture.md](architecture.md) for where the STF fits in the node.

| Spec | Rules |
| --- | --- |
| V1 | The rules new networks start with |
| V2 | V1's rules with tighter capacities (see [Capacities](#capacities)) |
| V3 | V2's rules with the fixes made after V2 froze (see [Frozen rules and V3](#frozen-rules-and-v3)) |

`OLGenesisParams` names the network's genesis spec as a required `genesis.spec`. New networks
use `1`; `2` starts a network under V2 and `3` under V3.

## Dispatch

[`crates/ol/stf`](../crates/ol/stf) is the one entry point to the STF. Each spec has a rule set
that implements the internal `OLStfSpec` trait (`v1.rs`, `v2.rs`, `v3.rs`), and the
`dispatch_spec!` macro in [`spec.rs`](../crates/ol/stf/src/spec.rs) maps each `OLSpecId` to its
rule set. The compiler makes every rule set provide every operation. Sequencer block assembly
composes the `sequencer` phases itself, so a rule set that changes their order must also
update [`crates/ol/block-assembly`](../crates/ol/block-assembly).

The state root records the spec, so every spec needs its own rule set even if it
changes no rules. V2 and V3 run the V1 code. The operations they share live in `capped_v1.rs`,
parameterized by the spec, the spec before it, its capacities, the rules it amends in the V1
code and the DA encodings it replays; each rule set calls them one operation at a time.

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

The OL state root is not the root of `OLStateV1`. Block headers, checkpoint terminal headers
and genesis commit to the hash tree root of `OLRootState`
([`crates/ol/state-types/ssz/root.ssz`](../crates/ol/state-types/ssz/root.ssz)):

- `cur_spec_version`: the spec that produced the chainstate. It picks the layout.
- `staged_spec_version`: always equal to `cur_spec_version`, since no rules stage a spec.
- `chainstate_root`: the root of the chainstate in its layout, `OLStateV1` today.

`OLRootState::compute_state_root` computes it.

The first block of the first V2 epoch after a V1 state sets `cur = staged = V2` in
epoch-initial processing, so every driver, DA replay included, reaches the same root. The
first V3 epoch takes over a V2 state the same way, as `cur = staged = V3`.

Genesis construction sets the initial versions (`new_genesis`). After that only the rules
change them, through `IStateAccessorMut::set_spec_versions` and the `WriteBatch` version slot.

### Layouts and the state container

`OLStateLayout::for_spec`
([`crates/ol/state-container/src/layout.rs`](../crates/ol/state-container/src/layout.rs)) maps
`cur_spec_version` to a chainstate layout. V1, V2 and V3 all use `OLStateV1`. Spec and layout
numbers are never compared: several specs can share a layout.

`OLStateContainer` ([`crates/ol/state-container`](../crates/ol/state-container)) pairs the root
with an `OLStateSeries` chainstate. Storage, the chain worker and the checkpoint proof input
carry it. It has no wire format of its own: serde writes the root's fields and the
chainstate's SSZ bytes (CBOR in sled, the zkVM's serde input in the proof), and decoding checks the
chainstate against the root. Execution uses `MemoryStateBaseLayer<OLStateV1>`, built with
`from_container` and turned back with `into_container`.

## Capacities

The V1 types bound transaction effects and block logs at 65,536 entries and message payloads
at 1 MiB. Those bounds fix V1 txids and encodings, so they cannot shrink. V2 adds tighter
capacities as validity rules instead: 255 transfers and 255 messages per transaction, and
4,096 logs per block. V3 keeps them and bounds each message payload at 4 KiB, the most the
epoch DA encodes for an inbox message.
`spec_limits` ([`crates/ol/stf/src/limits.rs`](../crates/ol/stf/src/limits.rs)) returns a
spec's capacities for the drivers that build blocks or admit transactions.

## Frozen rules and V3

A spec's rules freeze when its checkpoint program is published. The program's verifying key is
consensus, so a node must accept exactly the blocks and DA the program proves, no more and no
less. The rules include what the program's dependencies accept: `strata-codec` decoding,
`strata-merkle` proof verification, predicate verification and the DA decoders.

So a change that makes shared code stricter, more lenient or compute another result is a rule
change for every spec that runs the code, even when it arrives through a dependency update.
The code keeps the earlier behavior for the frozen specs and a later spec turns the new one
on. V0 is frozen at the 0.3.0 release, V1 at `v0.4.0-rc.1` and V2 at `v0.5.0-rc.2`.

V3 turns on the fixes made after V2 froze:

| Rule | V1 and V2 | V3 |
| --- | --- | --- |
| Message payload capacity | 1 MiB, the SSZ bound | 4 KiB |
| Descriptor length of a withdrawal message | any varint layout | the shortest layout |
| Accumulator proof for an inbox or L1 block ref claim | the proof's length selects the peak, and the index is checked within it | the proof leads to the peak that holds the claimed index |
| Epoch DA diff | every encoding the decoders read | the encoding the encoder writes |

- The capacity is in `OLSpecLimits::V3`.
- `RuleAmendments`
  ([`crates/ol/stf-v1/src/rules.rs`](../crates/ol/stf-v1/src/rules.rs)) selects the withdrawal
  and accumulator rules inside the V1 code. Without amendments the V1 code runs the frozen
  rules, and `strata-ol-stf` passes each spec's amendments on every call.
- `decode_ol_da_payload_bytes_canonical` in `strata-ol-da-types-v1` checks the DA encoding by
  re-encoding the decoded diff. The DA decoders themselves stay as lenient as the frozen
  programs.

Under V1 and V2 a snark account update can still prove an accumulator entry under an index in
another peak, so those specs keep that gap until a network moves to V3.

## Proofs per spec

The SP1 guest in [`provers/sp1`](../provers/sp1/README.md) proves one spec, fixed by
`PROVED_SPEC` in `guest-checkpoint/src/spec.rs`. It proves V3 today. V1 epochs keep using the
bundle published from `v0.4.0-rc.1` and V2 epochs the bundle published from `v0.5.0-rc.2`.

No rule set stages its successor, so the first epoch of a spec starts from a terminal state
of the spec before it. Proving a spec from V2 on therefore accepts a start state with
`cur = staged` at its predecessor as well as states of the spec itself: V2 proving accepts a
V1 state and V3 proving a V2 state. ASM key handover and the verified-tip checks limit that
exception to the first accepted epoch of the new spec. A program takes over only the spec
right before its own, so the V3 program refuses a V1 state.

SP1 proving of V3 needs a configured V3 bundle; tests use one built from the checkout. If a
node first needs V3 after it started and has no V3 artifact, proving waits. Startup fails if
an active or enacted-pending predicate has no matching artifact.

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
   `limits.rs`. Put each rule it changes behind the new rule set, as
   [Frozen rules and V3](#frozen-rules-and-v3) describes, and leave the earlier specs as they
   are.
3. Map it to a layout in `OLStateLayout::for_spec`. A new chainstate layout also needs a new
   `OLStateSeries` variant.
4. Decide how its first epoch takes over the previous spec's state, as V1, V2 and V3 do.
5. Move `PROVED_SPEC` to the new spec, publish its bundle and keep the previous spec's bundle
   frozen. Add the new spec to the native registry in `crates/ol/checkpoint-artifacts`.
6. Cover the handover with STF tests, and keep the golden V1 test in
   [`crates/ol/checkpoint/src/tests/golden_v1.rs`](../crates/ol/checkpoint/src/tests/golden_v1.rs)
   passing unchanged.
