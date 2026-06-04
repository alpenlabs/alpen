# VK (Predicate) Upgrade Architecture

Status: draft / design discussion
Scope: ASM, OL, and EE verifying-key (VK / predicate) upgrades
Related: STR-3480 (seal a batch with L1 view up to the enactment block)

## Purpose and motivation

Each of Alpen's three state-transition functions (ASM, OL, EE) is enforced by a
zero-knowledge proof verified against a **verifying key** (a `PredicateKey`).
Upgrading the logic of any layer therefore means changing the VK that the layer
above uses to verify it:

| Layer | VK held by (verifier) | What it verifies |
|-------|-----------------------|------------------|
| ASM   | the ASM client itself (L1-anchored) | ASM STF transitions |
| OL    | the ASM (checkpoint subprotocol) | OL-STF checkpoint proofs |
| EE    | the OL (snark-account `update_vk`) | EE account update proofs |

This document records *why* upgrades are structured the way they are, the
design principles we converged on, the alternatives we rejected, and the
recommended path forward for each layer.

## Why a controlled upgrade is hard

A VK is a **pure function of the ELF bytes**: any change to the guest program —
whether a one-character typo fix, a bug fix, or an entirely new codebase —
produces a new program ID and therefore a new VK. There is no such thing as a
"free" logic change that leaves the VK untouched. Consequently:

* The upgrade mechanism must be **uniform**: the process cannot depend on how
  large or small the underlying change is, because the VK change looks identical
  in every case.
* Because the VK gates verification, the moment of activation determines *which
  logic is considered valid for which blocks*. Getting that moment wrong is a
  consensus failure, not a cosmetic one.

## The governing rationale: the exit guarantee

The reason upgrades are **delayed** rather than applied immediately is the user
**exit guarantee**: a user who dislikes an announced upgrade must be able to
withdraw their funds under the *old* (currently trusted) rules before the new
rules take effect.

This single requirement drives most of the design:

1. The delay between **announcement** and **activation** is the user's exit
   window. It is a *safety floor*: it may be lengthened (e.g. by a lagging
   sequencer) but must never be shortened.
2. The activation point must be **observable on L1** and **not movable by the
   sequencer**, because users decide to exit by watching L1 and they rely on the
   window being honored.
3. Everything a user does on L1 *before* activation (notably a forced-inclusion
   or deposit/withdrawal transaction) must be processed under the **old** logic.

For the choice to exit to be *meaningful*, the new **code** — not just the VK —
must be published at announcement: a VK is opaque, so users need the source
(ideally via reproducible builds) to decide whether to object. Full nodes need
only the VK to verify; only provers need the ELF.

## Safety and liveness

The exit guarantee sorts every concern below into two classes, and the whole
design leans on keeping them apart:

* **Safety** (must *always* hold): the exit window is never shortened, and a
  proof never verifies under the wrong VK.
* **Liveness** (not guaranteed by the protocol alone): the upgrade eventually
  takes effect and the chain keeps progressing — contingent on a functioning
  sequencer. Its worst case is a *safe halt*, never a safety breach.
    * Liveness is not intrinsically enforceable — no rule makes a stalled
      sequencer *produce* a proof.

So the design should only ever let a misbehaving sequencer **delay** activation,
and delay only *lengthens* the exit window. Whatever the sequencer does is then a
**liveness** problem, never a **safety** one. The design leans on that — safety is
secured unconditionally (it falls out of anchoring on L1, below), while liveness
is left to sequencer rotation.

> **Note — critical bug-fixing is out of scope.** Enacting faster than the exit
> window would *shorten* it (a safety violation), so this path is the wrong tool
> for an actively-exploited bug. That is acceptable: a VK change is opaque, and
> funds ultimately sit under the bridge's **1/N** assumption, where an out-of-band
> correction is the last-resort backstop. No emergency-halt primitive is specified
> here.

## Anchoring activation on L1

This is the first place `B` appears, so to be plain: **`B` is the activation
boundary — the single L1 block height at which verification switches from the old
VK to the new one.** Everything below is about choosing `B` and reading it.

**Why an activation point is needed.** Because the upgrade is delayed, some single
moment must separate "verify under the old VK" from "verify under the new VK."
Every node has to agree on that moment — otherwise they disagree on which VK
validates a given proof, a consensus split — and users must be able to see it,
because it ends their exit window.

**The candidates.** `B` has to be named in some clock, and there are two: an **L2
coordinate** (an OL/EE block height or an OL epoch) or an **L1 coordinate** (a
Bitcoin block height).

**Why not an L2 height or epoch.** It is by far the easiest to build — each STF
already tracks its own height, so activation is a local `if height >= N` check
(the Ethereum fork-by-height model), with none of the L1 plumbing below. But an L2
clock is paced by the sequencer, and that is fatal:

* **The exit window can be shrunk.** The sequencer sets how fast L2 blocks are
  produced, so it can reach the activation height in an hour instead of two weeks.
  A *shorter* window is a safety failure, not an inconvenience.
* **The chain can be forced to halt.** Activation is defined to happen *at* height
  `N`, but the sequencer keeps producing blocks, so the chain can finalize past
  `N` — to `N+k` — under the old VK before the switch takes effect. The rule then
  cannot be honored: the new VK should apply from `N`, yet blocks `N..N+k` are
  already final under the old one and cannot be undone. The upgrade can no longer
  be applied, and the only way out is to halt.

**Why L1 works.** A Bitcoin height advances independently of the sequencer
(≈ 10 min/block), so define

```
B = (L1 inclusion height of the update tx) + 2016     (≈ 2 weeks)
```

`B` is a fixed wall-clock deadline the sequencer **cannot compress**, every node
computes it identically, and users can watch it. Because `B` is known a full
difficulty period before it arrives, the switch is always enforceable in time —
no node finalizes past `B` under the old VK, so there is nothing to unwind. The
sequencer can still *lag* (cross `B` late), but lag only *lengthens* the window,
which is safe.

**Nothing extra goes in the payload.** `B` is fixed the moment the update tx
lands, so the payload carries only the new VK; an explicit activation height would
be redundant. This is the authorization-vs-activation split made concrete: the L1
transaction is the **authorization** ("what"), and `inclusion + 2016` is the
**activation** ("when") — chosen by neither the admin nor the sequencer.

**How `B` is read — and why safety is unconditional.** Each node enforces `B`
with no trust in any other:

1. derive `B = inclusion + 2016` independently from L1;
2. read the **L1 view** `V` each proof commits to (the L1 blocks it incorporated),
   checked against the real L1 chain so `V` cannot be forged;
3. verify under the new VK iff `V ≥ B`, else the old VK; reject anything that does
   not verify under the rule-selected key.

This cuts **both** ways: a post-`B` proof under old logic is rejected (no late
switch) and a pre-`B` proof under new logic is rejected (no *early* switch — the
part that protects the exit window). The boundary is read from the L1 view *in the
proof*, never from a node's own L1 tip: a verifier can run ahead of the proof
stream it checks (the ASM sits at the L1 tip while the checkpoint stream it
verifies lags), and keying off its own tip would judge a proof against the wrong
VK. The per-layer plan works this "two-clock" point through on the OL.

## One live VK per layer

At any L1 point, exactly **one** VK is authoritative for a given proof. Across a
transition the verifier may briefly hold both — the ASM keeps the old and new
checkpoint predicates until the L1 view crosses `B`; the EE carries the incoming
VK as a queued inbox message — but it is only ever a transient pair, resolved at
the boundary:

* The swap lands at a **non-straddling proof boundary** (the checkpoint that ends
  exactly at `B` for OL; the update that ends exactly at the VK-update message for
  EE). Proofs are verified in order, so the verifier checks the last old-logic
  proof, swaps, then checks the first new-logic proof.
* **Historical** verification (fresh sync / replay) recovers the then-current VK
  by *replaying* the upgrade events in order — the old VK is derived by replay,
  not retained indefinitely.

(One axis stays out of this mechanism: behavioral changes the active ELF can
already express by height — an EVM hardfork the guest knows about — fire *inside*
the ELF with the **same VK** and need no rollover at all. Only changes that
produce a new ELF, hence a new VK, use the activation machinery above.)

## Lifecycle of an upgrade

This section ties the pieces above into one timeline. Nothing here is new design.

### Steps

1. **Build.** A new ELF is compiled; the build fixes its predicate (VK).
2. **Approve.** The new VK goes to the admin multisig. The signers review it and,
   if enough agree, sign an update transaction and post it to L1.
3. **Queue.** The ASM admin subprotocol accepts the transaction and holds it for a
   configurable **confirmation depth** — planned `2016` blocks (≈ 2 weeks) for VK
   updates; depth `0` would bypass the queue. During this window the admin can
   still **cancel** the upgrade (`MultisigAction::Cancel`) if something is wrong.
4. **Activate.** When the queue elapses — at `B = inclusion + 2016` — the new VK
   becomes the live rule, applied per layer as in the per-layer plan below.

The queue window does double duty: it is both the admin's cancellation window and
the users' exit window, over the same `2016` blocks. The queue, the per-variant
depth, and cancellation all already exist in `alpenlabs/asm`; what is new is the
per-height activation that follows.

### Where the time goes

The total time from "new ELF exists" to "new VK governs blocks" is the sum of
four delays:

| Stage | What it is | Whose clock |
|-------|------------|-------------|
| **Approval** | Reviewing the VK and collecting enough signatures | admin multisig |
| **Inclusion** | Getting the signed transaction into an L1 block | L1 mempool |
| **Queue** | The `2016`-block hold by the ASM (also the cancel/exit window) | Bitcoin |
| **Activation** | The sequencer crossing `B` in its proof stream | sequencer |

The first two happen before the window opens; the queue *is* the exit window; and
activation comes after it. That last stage exists because the sequencer reads
activation from the L1 view in its proofs and runs behind the L1 tip (to avoid L2
reorgs), folding in L1 data only at batch boundaries — so it adds a little more
time. Crucially it can only ever push activation *later*, never earlier, so it
never shortens the exit window. Its size is bounded by the per-layer budgets
(about one batch — one checkpoint at the OL, one EE batch at the EE).

## Per-layer implementation plan

The ASM admin and checkpoint subprotocols — the queue, the update actions, and
`CheckpointState` — live in the external
[`alpenlabs/asm`](https://github.com/alpenlabs/asm/tree/v0.1-alpha.10) repo
(pinned at `v0.1-alpha.10` in
[`Cargo.toml`](https://github.com/alpenlabs/alpen/blob/55907ce/Cargo.toml)); the
local repo holds the OL/EE state, the sequencer, the full-node verification
mirror, and the tooling. Much of the machinery **already exists** in `asm`: a
generic admin queue with a configurable per-variant **confirmation depth**
([`QueuedUpdate`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/subprotocol/src/queued_update.rs),
height-gated drain), **cancellation** during that window
([`MultisigAction::Cancel`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/txs/src/actions/cancel.rs)),
and the VK update actions
[`UpdateAction::{OlStfVk, AsmStfVk, EeStfVk, Sequencer}`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/txs/src/actions/updates/mod.rs).
What is *missing* is the per-height activation (dual predicate + boundary `B`):
the predicate is swapped immediately today. (`2016` is the *planned* depth for VK
updates — config, not a constant; depth `0` bypasses the queue.)

Code references are GitHub permalinks pinned to
[`55907ce`](https://github.com/alpenlabs/alpen/tree/55907ce) (Alpen) and
[`v0.1-alpha.10`](https://github.com/alpenlabs/asm/tree/v0.1-alpha.10) (asm).

### ASM

ASM is the simplest layer: it is a reactive state machine over L1 blocks, its STF
advancing one L1 block at a time, so the switch can be made at the L1 block
itself. Its own VK update already has an action
([`AsmStfVk`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/txs/src/actions/updates/asm_stf_vk.rs),
which emits an `AsmStfUpdate` log on enactment); the new behavior is in the STF:

1. **Stop at the boundary.** Once the queued update matures, the ASM knows the VK
   has changed and must stop applying the old STF to blocks past that point.
2. **Resume under explicit approval.** Continuing requires an explicit signal that
   the new logic is the one to run from here; how it is expressed depends on the
   change.

(The local CSM worker that consumes ASM output —
[`processor.rs::process_asm_block`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/csm-worker/src/processor.rs#L253)
— walks heights sequentially with no pause/halt; a real halt lives in the ASM
STF.) See the
[ASM Upgrade Strategy](https://app.notion.com/p/ASM-Upgrade-Strategy-319901ba000f8083af11dc33d88f0de9)
for the detailed plan.

### OL

**The problem today.** The `OlStfVk` update enacts at the end of the queue and
relays `UpdateCheckpointPredicate` to the checkpoint subprotocol, which swaps the
predicate **immediately and unconditionally**
([`update_checkpoint_predicate`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/checkpoint/verification/src/state.rs)).
Nothing then forces the OL sequencer to have processed every L1-originated
transaction before `B` under the old logic — it can do whatever it likes. That
breaks the exit guarantee and must change.

**Two predicates in `CheckpointState`.** The checkpoint predicate lives in the
ASM's
[`CheckpointState`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/checkpoint/verification/ssz/state.ssz)
— an SSZ-codegen'd container that today holds a *single* `checkpoint_predicate`. It
must hold **both the old and the new at once** (a schema change), from enactment
until activation. The boundary `B` is the L1 height at which the
`UpdateCheckpointPredicate` message arrives — i.e. the queue's `activation_height`.
Activation happens when a verified checkpoint's L1 view reaches `B`; the ASM then
drops the old predicate and verifies all later checkpoints under the new one. (This
refines the *one live VK* note above: at any L1 point one VK is authoritative, but
the state carries both across the transition.)

**The cut must be exactly at `B`.** The checkpoint subprotocol must reject any
checkpoint whose L1 range *straddles* `B`: a checkpoint carrying the tip from, say,
`B-1` to `B+1` is invalid — the break has to fall exactly on `B`. Otherwise the
sequencer could get `B+1` accepted under the old VK when it should be the new one.
The verification already commits to an exact inclusive L1 range:
[`verify_progression`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/checkpoint/verification/src/verification.rs)
returns `CheckpointL1Range::Range { start_height, end_height }` and binds the
manifest hashes over exactly that range (mirrored locally in
[`checkpoint_extract.rs`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/csm-worker/src/checkpoint_extract.rs#L122)).
So the data is there; the new rule — reject a checkpoint whose range straddles `B`
— goes into the checkpoint subprotocol's verification.

**Batch sealing must target `B`.** Sealing is driven purely by L2 slot count
([`epoch_sealing.rs` `FixedSlotSealing`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/block-assembly/src/epoch_sealing.rs#L23)),
with no L1-height input;
[`block_assembly.rs::fetch_asm_manifests_for_terminal_block`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/block-assembly/src/block_assembly.rs#L367)
pulls whatever L1 has produced, capped by a count. To honor the exact cut the
sequencer must seal the epoch so its L1 range ends exactly at `B`: add an
L1-height-aware seal condition to the policy and `construct_block`, and bound the
manifest fetch to end at `B`.

### Alpen EE

**The problem today.** At the end of the queue the `EeStfVk` enactment emits an
`EePredicateKeyUpdate` log targeting the Alpen EE account
([`relay_alpen_predicate_update`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/subprotocol/src/handler.rs)).
The OL processes it at its epoch-terminal block and swaps the snark account's
`update_vk` immediately
([`process_ee_predicate_key_update`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/stf/src/manifest_processing.rs#L317)
→ [`set_update_vk`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/state-types/src/snark_account.rs#L77)).
As with OL, nothing forces the sequencer to process the pre-`B` L1-originated
transactions under the old logic, so the invariant — *everything a user does on L1
before activation runs under the old logic* — is violated. (`B` here is the log's
landing height, the queue's `activation_height`.)

**Why the OL approach does not transfer.** The OL anchors the switch to the L1
view it commits in each checkpoint. The EE has **no L1 view of its own**: its
account state
([`EeAccountState`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ee-acct-types/src/state.rs#L9))
tracks only execution tips and pending inputs, and an update commits L1 only as
`ledger_refs` (references to DA blocks, not a tracked cursor). We *could* give the
EE an OL view, but we deliberately **do not anchor EE activation to OL height**:
the OL is not yet robust, and a faulty OL could shorten the EE exit window — a
safety failure.

**Use the inbox-MMR ordering.** Every L1-originated message a snark account
receives is appended to its **inbox MMR** in L1 order
([`insert_inbox_message`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/state-types/src/snark_account.rs#L70);
deposits travel
[`process_deposit_log`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/stf/src/manifest_processing.rs#L231)
→ `process_message` →
[`handle_snark_msg`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/snark-acct-sys/src/handlers.rs#L6)).
That ordering is the boundary:

1. **Queue, don't swap.** When the VK-update log arrives, the OL inserts a
   **VK-update message** into the inbox MMR — in its L1-order position among the
   deposits — instead of calling `set_update_vk`.
2. **Terminate the batch at it.** The EE account guest must **end the update
   exactly at the VK-update message** when one is present: every message ahead of
   it (the pre-`B` L1 transactions) is processed under the old logic, and the next
   update begins under the new one.
3. **Carry it in the OLTx.** The snark-account-update transaction already carries
   the processed inbox messages
   ([`SauTxOperationData.messages`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/chain-types/src/transaction.rs#L152)),
   so the VK-update message rides along, and the OL applies the swap when it sees
   that message processed as the batch's last.

Today the guest stops by **message count**, not by a designated message
([`program_processing.rs`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/snark-acct-runtime/src/program_processing.rs#L47)
checks `cur_next_msg_idx + msg_count == new_next_msg_idx`), so "terminate exactly
at the VK-update message" is a new rule.

**Two properties worth noting:**

* **EE upgrades become EE-defined.** Because the update is just a message the EE
  account interprets, different EEs can choose different upgrade semantics and
  invariants. The user-exit guarantee is *Alpen's* choice, not a protocol-wide
  one.
* **The update could originate from another EE** via cross-account messaging — it
  is up to each EE to define the message and its rules. For Alpen specifically,
  the VK-update message must therefore be verified to have **originated from L1**
  (the ASM), not from the sequencer or another account.

**What changes:**

* **EE message type & batch rule** — add a VK-update variant to
  [`DecodedEeMessageData::decode_raw`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ee-acct-types/src/messages.rs#L39)
  / `apply_decoded_message`, and enforce termination on it in
  [`ee_program.rs::finalize_verification`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ee-acct-runtime/src/ee_program.rs#L123)
  /
  [`verify_update_inner`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/snark-acct-runtime/src/program_processing.rs#L91)
  (likely via a new `UpdateExtraData` marker).
* **OL transaction (OLTx)** — recognize the terminal VK-update message in
  [`process_update_tx`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/stf/src/transaction_processing.rs#L102)
  and apply `set_update_vk` only then, checking the message's L1/ASM provenance.
* **OL log handling** — change
  [`process_ee_predicate_key_update`](https://github.com/alpenlabs/alpen/blob/55907ce/crates/ol/stf/src/manifest_processing.rs#L317)
  to *insert the inbox message* instead of swapping.

### Alpen EE: EVM forks and the L2-height coordinate

The inbox-MMR boundary decides *which VK verifies the account updates*, but for
Alpen it is not the whole story. Alpen's EE is EVM-based, and most upgrades are
**EVM forks** activated through the **reth chainspec**
([`crates/reth/chainspec`](https://github.com/alpenlabs/alpen/tree/55907ce/crates/reth/chainspec),
[`crates/reth/evm`](https://github.com/alpenlabs/alpen/tree/55907ce/crates/reth/evm)).
A reth fork activates at a hardcoded **L2 block height** (or timestamp) in the
chainspec, and the chainspec is **part of the ELF** — so the fork height `H` is
baked into the new guest at build time, *before* the VK is even authorized. (This
is the "new ELF" case: a fork decided now is not pre-baked in the active ELF, so
it needs both the VK rollover *and* a chosen `H`.)

That reintroduces the L2-timing problem we banished for the VK switch. The switch
itself stays L1-anchored and safe (the inbox-MMR boundary), but the EVM fork fires
at L2 height `H`, and whether the chain reaches `H` *after* the new ELF is active
depends on the sequencer's block rate — which the sequencer controls. The
invariant: **the chain must not produce block `H` until the new ELF (the one that
knows about the fork at `H`) is active.** If the old ELF is still active when the
chain reaches `H`, it produces `H` under the old rules and the fork is silently
missed — a safety problem.

**So `H` must be chosen first, before anything else,** and with margin: it is
committed at the very start of a long pipeline (in the ELF, built before
authorization, before the 2-week enactment). You are forecasting where the EE
chain will be ~two weeks out and setting `H` safely beyond that.

**Back-of-the-envelope** (EE block time 5 s):

| Delay | Estimate |
|-------|----------|
| Admin (agree, sign, broadcast) | ~1 day (variable) |
| Enactment (`2016` BTC blocks) | ~14 days |
| OL application (ASM logs applied only at the epoch-terminal block) | ~9 h batch |
| EE sequencer (≈ 4 h batch + ~6-block L1 lag + acts only on finalized checkpoint state) | ~1 day |

Total ≈ **~16 days**, or at 5 s/block ≈ **~276k EE blocks**. So `H` should sit at
least that far beyond the current EE tip, plus a safety margin.

**Direction of the risk.**

* **Sequencer slower than planned → safe.** The chain reaches `H` later, so the
  new ELF is comfortably active first; the window only grows.
* **Sequencer faster than planned → potentially unsafe.** The chain could reach
  `H` before the VK switches. During an upgrade we therefore recommend: **control
  the sequencer, monitor it closely, and if needed adjust the block time** — a
  *config* change only, nothing else in the ELF or rules.

**If a stage runs long.** The same forecasting risk applies whenever a delay
stretches:

* **Admin can't reach threshold** → the whole pipeline shifts later, so the chain
  is further along by activation; the chosen `H` must have had margin for it.
* **OL goes down (unrelated reasons)** → the EE sequencer, which advances only on
  finalized OL/checkpoint state, stalls or skews and must **re-adjust its block
  timing** to still land the fork at `H` after activation.

### Cross-cutting: rotation, live predicate reads, burial

* **Live predicate reads (prerequisite).** Today `sequencer_predicate` /
  `checkpoint_predicate` are read **once at startup** and cloned into the FCM
  context (`bin/strata/src/fcm.rs`; `crates/consensus-logic/src/fcm`). For any
  ASM-driven swap (OL VK activation *or* sequencer rotation) to take effect
  without a restart, the node must re-read the predicate from the live ASM
  checkpoint section. This is a shared prerequisite.
* **Sequencer rotation (liveness backstop).** Rotation already has an admin action
  —
  [`UpdateAction::Sequencer`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/txs/src/actions/updates/mod.rs)
  swaps the `sequencer_predicate` — and block-producer auth already checks that
  predicate (`crates/consensus-logic/src/fcm/service.rs::check_ol_block_proposal_valid` →
  `crates/ol/chain-types/src/validation.rs::verify_sequencer_predicate_signature`).
  To apply **immediately** (no delay, since it changes the operator, not the
  rules), give the `Sequencer` variant a confirmation depth of `0` (which bypasses
  the queue). The one local gap is the live-read above — the predicate is cached at
  startup today.
* **Burial.** `B`'s enforcement uses the existing `l1_reorg_safe_depth`; since
  `2016 ≫` that depth, the enactment boundary is final by the time anyone acts.

### Build, ELF distribution, and rollout order

The new source/ELF is published at announcement; full nodes need only the VK to
verify, provers need the ELF (`provers/sp1/build.rs` → `vks.rs`,
`crates/zkvm/hosts`). A workable order: (1) external `alpenlabs/asm` changes
(dual predicate + `B` + exact-cut in `CheckpointState`; the queue, depth, and
actions already exist) and re-pin; (2) live predicate reads;
(3) OL exact-cut sealing (STR-3480) + `csm-worker` mirror; (4) EE VK-update
message type + guest batch-termination rule + OLTx handling; (5) rotation tooling.
Each step is independently testable.

## Alternatives considered and rejected

| Alternative | Verdict |
|-------------|---------|
| **Overwrite the VK as soon as the authorization is applied** | Rejected: activation timing non-deterministic, not announceable, no exit window, sequencer cannot coordinate. |
| **OL-derived activation** (e.g. "+D epochs after the log applies") | Rejected: absolute activation still hostage to L1/checkpoint timing; `D` is a magic constant; not announceable when authorized. |
| **Per-EE-block-height VK schedule** | Rejected: breaks the one-proof-one-VK invariant; block-height rule changes belong *inside* the ELF (height-conditional logic in the guest), not in the VK schedule. |
| **"Bake everything into the ELF, never change the VK"** | Impossible as a complete solution: the VK is a function of the ELF, so a new fork height *is* a new VK. Useful only for forks pre-baked into the ELF at authorization time; cannot authorize a genuinely new prover program. |
| **Activate OL VK purely by Bitcoin block height** | Rejected: the major/minor two-clock problems of the OL plan — batch replay and node-timing divergence. |
| **Anchor EE activation to an OL view / OL height** | Rejected: ties the EE exit window to the OL, which is not yet robust — a faulty OL could *shorten* the window (a safety failure). Superseded by inbox-MMR ordering (the EE plan). |
| **One emergency mechanism that also does fast bug fixes via the upgrade path** | Rejected: category error — the exit delay and fast bug-fixing are different classes. Critical bugs are out of scope (bridge 1/N backstop), not a VK-upgrade concern. |

## Open questions

* **STR-3480 exact cut:** the OL requires the checkpoint to break exactly at `B`;
  confirm the sequencer can seal there reliably (an off-cadence short epoch may be
  needed). See the OL plan.
* **`CheckpointClaim` binding:** confirm that envelope + ASM L1-check binding is
  sufficient so the L1 range need not enter the proven claim (recommended). See
  the OL plan.
* **EE-from-L1 / provenance:** the VK-update message lives in OL state and DA, so
  an L1-syncing EE node sees it by verifying OL; confirm that suffices, and pin
  down how the EE proves the message **originated from L1/ASM** (not the sequencer
  or another account). See the EE plan.
* **EE anti-stall deadline:** whether to require the snark update to land within
  ~3 L1 blocks of `B` (with a commit→prove flow). Deferred — high lift.
* **Admin-update tooling (local):** the asm actions all exist (`OlStfVk`,
  `AsmStfVk`, `Sequencer`, `Cancel`), but Alpen-side tooling
  (`bin/strata-test-cli`) builds only `EeStfVk` — the rest need CLI support.
* **Critical-bug handling:** confirm the bridge 1/N + out-of-band-correction
  backstop is acceptable for the threat model (no emergency-halt primitive).
* **Forced inclusion:** not yet implemented; the interim liveness backstop is
  immediate sequencer rotation.

## References

* STR-3480 — seal a batch with L1 view up to the enactment block.
* **OL:** `crates/checkpoint-types/src/batch.rs` (`BatchInfo.l1_range`,
  `EpochSummary.new_l1`); `crates/ol/checkpoint/src/state.rs` (`CheckpointTip`);
  `crates/csm-worker/src/checkpoint_extract.rs` (`verify_checkpoint`);
  `crates/ol/block-assembly/src/{epoch_sealing.rs,block_assembly.rs}` (sealing);
  `crates/proof-impl/checkpoint/src/statements.rs` (`CheckpointClaim`).
* **EE:** `crates/ol/stf/src/manifest_processing.rs`
  (`process_ee_predicate_key_update`);
  `crates/ol/stf/src/transaction_processing.rs` (SAU apply / swap gate);
  `crates/ol/stf/src/proof_verification.rs` (verify vs `update_vk`);
  `crates/snark-acct-types/src/update.rs` (`LedgerRefs` = L1 heights);
  `crates/ol/state-types/{ssz/state.ssz,src/snark_account.rs}` (snark state);
  `crates/ol/da/src/types/{ledger.rs,payload.rs}` (OL DA);
  `bin/alpen-client/src/{main.rs,prover/spec_acct.rs}` (ELF/host binding).
* **ASM / cross-cutting:** `crates/state/src/asm_state.rs`;
  `crates/params/src/lib.rs` (`checkpoint_predicate`, `cred_rule`,
  `l1_reorg_safe_depth`); `bin/strata/src/fcm.rs` (predicate read-once gap);
  `crates/consensus-logic/src/fcm/service.rs` +
  `crates/ol/chain-types/src/validation.rs` (block-producer auth);
  `bin/strata-test-cli/src/cmd/create_ee_predicate_update.rs` (admin-update tooling).
* **`alpenlabs/asm`** (pinned `v0.1-alpha.10`): admin subprotocol —
  [`handler.rs`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/subprotocol/src/handler.rs),
  [`state.rs`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/subprotocol/src/state.rs),
  [`queued_update.rs`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/subprotocol/src/queued_update.rs),
  [actions](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/admin/txs/src/actions/updates/mod.rs)
  (`OlStfVk`/`AsmStfVk`/`EeStfVk`/`Sequencer`/`Cancel`),
  [`confirmation_depth.rs`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/params/src/subprotocols/admin/confirmation_depth.rs);
  checkpoint subprotocol —
  [`state.ssz`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/checkpoint/verification/ssz/state.ssz)
  (`CheckpointState`),
  [`verification.rs`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/checkpoint/verification/src/verification.rs)
  (`verify_progression`, `CheckpointL1Range`),
  [`state.rs`](https://github.com/alpenlabs/asm/blob/v0.1-alpha.10/crates/subprotocols/checkpoint/verification/src/state.rs).
* SPS-60 (Moho), SPS-62/63 (checkpoint), SPS-64 (bridge) — protocol context.
