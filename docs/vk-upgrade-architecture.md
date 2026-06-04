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

We keep **one** active VK per layer in state at any instant, mirroring the OL
checkpoint predicate's single-slot model — old and new are never stored
simultaneously:

* The swap happens atomically at a **non-straddling proof boundary** (an OL epoch
  for the checkpoint VK; an EE batch for the EE VK). Because proofs are verified
  in order, the verifier checks the last old-logic proof, swaps, then checks the
  first new-logic proof — there is never an instant where two VKs are needed.
* **Historical** verification (fresh sync / replay) recovers the then-current VK
  by *replaying* the upgrade events in order. "One slot in state" does **not**
  mean the old VK is never needed again; it means it is derived by replay, not
  retained.

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
   fixed number of L1 blocks (planned: `2016`, ≈ 2 weeks). During this window the
   admin can still **cancel** the upgrade if something turns out to be wrong.
4. **Activate.** When the queue elapses — at `B = inclusion + 2016` — the new VK
   becomes the live rule, applied per layer as in the per-layer plan below.

The queue window does double duty: it is both the admin's cancellation window and
the users' exit window, over the same `2016` blocks.

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
(about one batch at the OL, plus the one-update wiggle at the EE).

## Per-layer implementation plan

Two facts shape every layer. First, the ASM core — the admin subprotocol that
queues and authorizes VK updates, `CheckpointState`, `AnchorState`, and the log
types — lives in the external `alpenlabs/asm` dependency (pinned in
`Cargo.toml`); the `2016`-block queue and dual-predicate selection are
implemented *there*, and the local repo consumes the result. Second, the L1 view
each VK decision needs is **already committed** in that layer's artifact (the
checkpoint's `CheckpointTip.l1_height` for OL; the snark update's `ledger_refs`
for EE), so most local work is *selection and sealing alignment*, not new
commitments. Each subsection gives the design recap, what already exists, and the
concrete changes grouped by component.

### ASM

**Design.** The ASM's own STF VK is the easy case: one client, one clock.
Activation is `inclusion + 2016`; the verifier *is* the L1 follower, so there is
no second clock and the single slot holds directly.

**Already there / external.** All of this lives in `alpenlabs/asm`: the admin
subprotocol (ID 0), the queue, and the swap. Locally the node only consumes
`AnchorState` via `crates/state/src/asm_state.rs`. There is **no** local
activation concept to build.

**Changes.**

* *External (`alpenlabs/asm`):* the `2016`-block queue and the dual-slot swap for
  the ASM/Moho VK; re-pin the tag in `Cargo.toml` afterward.
* *Tooling (local):* add an admin-update CLI for the ASM / checkpoint predicate
  alongside the existing `bin/strata-test-cli` `create-ee-predicate-update` —
  today that is the *only* admin-update command.

### OL

**Design.** The OL-STF VK is the *checkpoint predicate*, held in the ASM's
`CheckpointState`. Activate by the **L1 view committed in the checkpoint**: the
ASM verifies a checkpoint under the old predicate while its L1 view is below `B`,
under the new one at/after `B`. The sequencer seals the boundary epoch with L1
view up to `B-1` (STR-3480) so the cut is clean, and the single live slot is
preserved because checkpoints are verified in epoch order (last old, swap, first
new).

**Already there.** The decision input exists end to end: the on-chain
`CheckpointTip.l1_height()` (epoch-terminal L1 height,
`crates/ol/checkpoint/src/state.rs`) and the local `EpochSummary.new_l1` /
`BatchInfo.l1_range` (`crates/checkpoint-types/src/batch.rs`) already commit the
L1 view. The predicate is verified inside the ASM checkpoint subprotocol and
mirrored locally in `crates/csm-worker/src/checkpoint_extract.rs::verify_checkpoint`.
The proven `CheckpointClaim` (`crates/proof-impl/checkpoint/src/statements.rs`)
commits `l2_range` + `asm_manifests_hash` — not the L1 range directly, but the
manifests hash already binds the L1 blocks processed.

**Gap.** Sealing is driven purely by L2 slot count (`FixedSlotSealing` in
`crates/ol/block-assembly/src/epoch_sealing.rs`); there is no way to seal "up to
L1 height `B-1`." That is the core STR-3480 work.

**Changes.**

* *External (`alpenlabs/asm`):* `CheckpointState` holds `{old predicate, new
  predicate, boundary B}`; checkpoint verification selects old/new by the
  checkpoint's L1 view vs `B`; the admin subprotocol runs the `2016` queue.
* *Sequencer (local, STR-3480):* seal the boundary epoch at L1 view `B-1`. Add an
  L1-target seal condition to `epoch_sealing.rs`; in
  `crates/ol/block-assembly/src/block_assembly.rs` cap
  `fetch_asm_manifests_for_terminal_block` at `B-1` and feed it into `should_seal`
  in `construct_block`; thread a `target_l1_height` through `BlockGenerationConfig`
  (`types.rs`), `builder.rs`/`state.rs`, and the `crates/ol/sequencer` duty, which
  sources `B` from ASM state.
* *Full node (local):* mirror the same old/new selection in
  `crates/csm-worker/src/checkpoint_extract.rs` and `processor.rs` so local
  verification matches the ASM (and re-read the predicate live — see the
  cross-cutting notes).
* *Proving (optional, likely unnecessary):* bind the L1 range into
  `CheckpointClaim` only if the threat model requires the *proof* (not the signed
  envelope) to carry the L1 view. Recommended **not** needed: the ASM has direct
  L1 access and checks `CheckpointTip.l1_height` against its own view, and
  `asm_manifests_hash` already commits the manifests.

**Reading-rule choice (STR-3480).** Recommend the *relaxed* rule — an epoch whose
**start** L1 height (previous verified tip `+ 1`) is below `B` stays entirely on
the old VK, so a straddling epoch need not be re-cut. The ASM can derive the
start from its previous `verified_tip`, so no off-cadence short batch is forced.

**Application tail.** Of the delays in the lifecycle section, only the activation stage is
OL-specific: because L1 info is applied at the epoch-terminal block, the swap
lands batch-granularly — up to one batch (≈ 9 h, ~2.7 % of the window) after `B`.
It only *lengthens* the window (safe), and the reorg lag is subsumed
(`2016 ≫ l1_reorg_safe_depth`).

### EE

**Design.** The EE VK is the snark-account `update_vk` in OL state. Gate the swap
on the EE update's **own** committed L1 view — `max(ledger_refs.idx())`, the
highest L1 block the batch incorporated — versus `B`. Store a *pending* VK + `B`
in the snark account; verify each update under the current VK and, *after*
applying, swap to pending once the update's L1 view reaches `B`. Because
verification reads the VK before the swap, the update that crosses `B` is checked
under the **old** VK and the next under the **new** — this *is* the one-update
"wiggle room," and it absorbs proving-time jitter.

**Already there.** The EE update already commits its L1 view via
`ledger_refs: LedgerRefs` (each `AccumulatorClaim.idx()` is an L1 height), built
in production by `build_ledger_refs_from_da` (`crates/alpen-ee/.../update_builder.rs`).
The OL verifies the update against `update_vk` in
`crates/ol/stf/src/proof_verification.rs`. Deposits arrive separately, via the
inbox MMR.

**Gap.** The current `EePredicateKeyUpdate` path swaps **immediately and
unconditionally** at the epoch terminal (`process_ee_predicate_key_update`,
`crates/ol/stf/src/manifest_processing.rs:317`), and the ELF/VK is bound **once**
at prover startup (`bin/alpen-client/src/main.rs`), not per batch.

**Changes.**

* *OL state (local):* add `pending_update_vk` + `pending_activation_height` to
  `OLSnarkAccountState` (`crates/ol/state-types/ssz/state.ssz` +
  `snark_account.rs`), with accessors/mutators in `crates/ledger-types`, and
  encode them in OL DA (`crates/ol/da/src/types/{ledger.rs,payload.rs}`) so
  L1-reconstructing nodes apply the same swap.
* *OL STF (local):* `process_ee_predicate_key_update` stores `pending(new_vk, B)`
  instead of swapping, where `B` = the L1 height the log lands at (pass the
  `real_height` already available in `process_asm_log`; the ASM emits the log at
  enactment, after its queue, so no explicit height field is needed). Gate the
  swap in the SAU apply path (`crates/ol/stf/src/transaction_processing.rs`, the
  `update_account` closure): after applying, if `pending` is set and the update's
  L1 view `≥ B`, `set_update_vk(pending)` and clear.
* *EE sequencer (local):* select the ELF/host per batch by the batch's L1 view vs
  `B`. Hold both hosts + `B` at the prover boundary (`bin/alpen-client/src/main.rs`,
  `prover/spec_acct.rs`, which already has `da_refs`), route per task in
  `crates/prover-core`, and have `crates/alpen-ee/ol-tracker` surface the
  checkpoint's committed L1 view (it exposes epochs today). Aim to cut the batch
  at `B-1` under the old ELF and start the new ELF at `B`; the wiggle covers
  jitter. A commit→prove flow would decouple this from proving latency.
* *EE full node from L1 (local):* reconstruction applies the same gated swap;
  `crates/ee-acct-runtime/src/update_processing.rs` selects the chunk predicate
  key by `B`. This rests on the assumption that an L1-syncing EE node takes swap
  timing from verified OL state rather than re-deriving it (see the open questions).
* *External (optional):* add an explicit `activation_l1_height` to
  `EePredicateKeyUpdate` only if the ASM should emit the log at *announcement*
  rather than enactment; the default (emit at enactment, `B` = landing height)
  avoids any external field change.

### Cross-cutting: rotation, live predicate reads, burial

* **Live predicate reads (prerequisite).** Today `sequencer_predicate` /
  `checkpoint_predicate` are read **once at startup** and cloned into the FCM
  context (`bin/strata/src/fcm.rs`; `crates/consensus-logic/src/fcm`). For any
  ASM-driven swap (OL VK activation *or* sequencer rotation) to take effect
  without a restart, the node must re-read the predicate from the live ASM
  checkpoint section. This is a shared prerequisite.
* **Sequencer rotation (liveness backstop).** No rotation exists; the sequencer
  identity is the `sequencer_predicate` in the ASM checkpoint config. Block-
  producer auth already checks it
  (`crates/consensus-logic/src/fcm/service.rs::check_ol_block_proposal_valid` →
  `crates/ol/chain-types/src/validation.rs::verify_sequencer_predicate_signature`).
  Rotation = an external admin action that swaps `sequencer_predicate` (analogous
  to `EeStfVk`) + the live-read above; it applies **immediately** (no `2016`
  delay) because it changes the operator, not the rules.
* **Burial.** `B`'s enforcement uses the existing `l1_reorg_safe_depth`; since
  `2016 ≫` that depth, the enactment boundary is final by the time anyone acts.

### Build, ELF distribution, and rollout order

The new source/ELF is published at announcement; full nodes need only the VK to
verify, provers need the ELF (`provers/sp1/build.rs` → `vks.rs`,
`crates/zkvm/hosts`). A workable order: (1) external `alpenlabs/asm` changes
(dual-slot + `B` + queue + selection) and re-pin; (2) live predicate reads;
(3) OL sealing (STR-3480) + `csm-worker` mirror; (4) EE pending-VK state + DA +
gate + per-batch host selection; (5) rotation tooling. Each step is independently
testable.

## Alternatives considered and rejected

| Alternative | Verdict |
|-------------|---------|
| **Overwrite the VK as soon as the authorization is applied** | Rejected: activation timing non-deterministic, not announceable, no exit window, sequencer cannot coordinate. |
| **OL-derived activation** (e.g. "+D epochs after the log applies") | Rejected: absolute activation still hostage to L1/checkpoint timing; `D` is a magic constant; not announceable when authorized. |
| **Per-EE-block-height VK schedule** | Rejected: breaks the one-proof-one-VK invariant; block-height rule changes belong *inside* the ELF (height-conditional logic in the guest), not in the VK schedule. |
| **"Bake everything into the ELF, never change the VK"** | Impossible as a complete solution: the VK is a function of the ELF, so a new fork height *is* a new VK. Useful only for forks pre-baked into the ELF at authorization time; cannot authorize a genuinely new prover program. |
| **Activate OL VK purely by Bitcoin block height** | Rejected: the major/minor two-clock problems of the OL plan — batch replay and node-timing divergence. |
| **EE tracks L1 burial independently to time activation** | Workable with wiggle room, but requires trusting the OL sequencer's timing / risks OL-EE divergence. Superseded by the checkpoint-L1-view variant (the EE plan). |
| **One emergency mechanism that also does fast bug fixes via the upgrade path** | Rejected: category error — the exit delay and fast bug-fixing are different classes. Critical bugs are out of scope (bridge 1/N backstop), not a VK-upgrade concern. |

## Open questions

* **STR-3480 boundary alignment:** confirm the *relaxed* reading rule (epoch
  start `< B` ⇒ old) over an exact cut. See the OL plan.
* **`CheckpointClaim` binding:** confirm that envelope + ASM L1-check binding is
  sufficient so the L1 range need not enter the proven claim (recommended). See
  the OL plan.
* **EE-from-L1 assumption:** confirm an L1-syncing EE node can take swap timing
  from verified OL state rather than re-deriving it; if not, it must independently
  check `B` against the update's L1 view (both are available). See the EE plan.
* **EE anti-stall deadline:** whether to require the snark update to land within
  ~3 L1 blocks of `B` (with a commit→prove flow). Deferred — high lift.
* **External admin actions:** whether `alpenlabs/asm` exposes update actions for
  the *checkpoint predicate* and *sequencer predicate* (only `EeStfVk` is used
  locally) — needed for OL VK activation and rotation tooling.
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
* **External** (`alpenlabs/asm`, `strata-common`; pinned in `Cargo.toml`): admin
  subprotocol, `CheckpointState`, `AnchorState`, `EePredicateKeyUpdate`,
  `PredicateKey`, `L1Height`.
* SPS-60 (Moho), SPS-62/63 (checkpoint), SPS-64 (bridge) — protocol context.
