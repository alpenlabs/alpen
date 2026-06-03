# Verifying-Key (Predicate) Upgrade Architecture

Status: draft / design discussion
Scope: ASM, OL, and EE verifying-key (VK / predicate) upgrades
Related: STR-3480 (seal a batch with L1 view up to the enactment block)

## 1. Purpose and motivation

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

### Why a controlled upgrade is hard

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

### The governing rationale: the exit guarantee

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

## 2. Design principles

These principles emerged from analyzing the layers together; they apply
uniformly.

### P1. Separate *authorization* from *activation*

* **Authorization** ("this new VK is approved") is a governance/security
  decision. It is intrinsically slow and asynchronous and *must* go through the
  L1/ASM admin multisig.
* **Activation** ("from this point on, verification uses the new VK") is a
  consensus-timing decision. It should be fast to reason about, deterministic,
  and **pre-announced** — like an Ethereum fork height.

If the two are conflated — if activation happens whenever the authorization
*happens to* propagate through the layers — the timing becomes the convolution
of L1 landing → ASM queue → checkpoint delivery → application, which is neither
announceable nor controllable.

**The plan** removes that convolution by deriving the activation point from one
L1 fact plus a fixed constant: activation occurs a fixed delay (`2016` Bitcoin
blocks ≈ one difficulty period) after the L1 **inclusion height** of the
authorized update transaction. The payload carries *what* changes (the new VK);
the *when* is `inclusion_height + 2016`, computed identically by every node from
L1. (If a longer window is ever wanted for a riskier change, the payload could
carry an additional delay floored at the `2016`-block minimum; the default is the
fixed constant.)

Deriving the activation point this way — a fixed delay measured *forward from
actual inclusion*, rather than an absolute height the admin writes into the
payload — is what keeps the decoupling robust: the boundary cannot be "already
passed" or set too soon, it is fixed and public the instant the tx lands
(≈ two weeks of notice), and it is movable by neither the admin nor the
sequencer. The slow multisig/L1 path delivers the authorization; a fixed forward
delay decides *when*.

**Consequence — activation is sequencer-influenced, and that is intrinsic.**
Gating activation on the proof stream (P2) means the sequencer, which paces that
stream, also influences how promptly the running system *crosses* that L1-fixed
boundary; in the limit it can delay the switch (bounded as the enforcement note
below shows). This is not a wart to design away — you cannot have both
"activation independent of the sequencer" *and* "activation synchronized to the
proof stream." It is the dual of P6, and it is bounded three ways: the direction
is safe (delay only *lengthens* the exit window, P5), it is capped by sequencer
rotation (P6), and the urgent case is out of scope by construction (P5). The
clean long-term severance is forced inclusion.

#### Enforcing the split: safety vs. liveness

"Can activation be *enforced*?" is two questions with opposite answers.

**Safety — no early activation, no wrong-VK acceptance — is fully enforceable
and needs no trust between nodes.** The mechanism is the reading rule of P2:

1. Every node derives the boundary `B = inclusion_height + 2016` independently
   from L1.
2. Every proof commits its L1 view `V`, and `V` is checked against the real L1
   chain, so it cannot be forged.
3. Verify under the new VK iff `V` crosses `B`, else the old VK; reject anything
   that does not verify under the rule-selected key.

This cuts **both** ways: a post-`B` proof produced under old logic is rejected
(no late switch), and a pre-`B` proof produced under new logic is rejected (no
*early* switch — this is what actually enforces the exit window). Because `B` is
an L1 fact every node computes identically, the enforcement is decentralized.

**Liveness — forcing the switch to actually happen — is not intrinsically
enforceable.** No reading rule compels a stalling sequencer to *produce* a
post-`B` proof; this is the general sequencer-liveness problem (P6). But the
protocol narrows it sharply: the sequencer **cannot validly process new L1
content under old logic**. Past `B` its only options are (a) switch to the new
logic, or (b) freeze its L1 view — which stops crediting deposits, withdrawals,
and all L1-originated activity. There is no option to keep running normally while
quietly staying on old logic, so delay manifests as a **visible halt**, which is
exactly what triggers immediate rotation (P6).

**Optional hard rule.** A deadline ("if no proof with `V ≥ B` appears within `N`
L1 blocks of `B`, reject further old-logic proofs") turns the visible halt into a
consensus rule. It still cannot force activation — only force the stall to become
an explicit, rotation-triggering halt. Since freezing is already self-evident,
this may not be worth the lift.

**Open item — EE-from-L1.** For ASM and OL the verifier has direct L1 access and
enforces `B` itself. For EE, whether a node syncing from L1 must independently
check activation timing or may delegate it to the OL proof is unsettled (§3.3);
feasibility holds either way, only the *location* of the check changes.

### P2. Activation is gated on the *proof stream*, never on the verifier's own clock

This is the central correctness principle, and the subtlest.

Each layer's VK lives in the layer *above* it, and that verifier may run on a
**different clock** than the proofs it verifies. The ASM is at the L1 tip; the
OL checkpoint stream it verifies *lags* behind that tip. If the verifier swaps
its VK based on *its own* clock (e.g. "when my L1 tip reaches the enactment
height"), it will apply the new VK to a proof that was produced under the old
logic — because the proven content had not yet reached the boundary. The result
is either a stalled chain (proof fails) or a silent exit-guarantee violation
(old, pre-boundary messages retroactively governed by new logic).

> **Principle:** the verifier swaps the VK **synchronized to the proof stream it
> verifies**, gated on the *proven content* crossing the L1-anchored boundary —
> never on the verifier's own clock.

The concrete, shared coordinate we use for "the proven content's position" is
the **L1 view committed inside the proof** (the range of L1 blocks the
batch/checkpoint incorporated). Both producer and verifier read the same L1 view
from the same artifact, so they cannot diverge.

### P3. A single live VK slot per layer is sufficient

We deliberately keep **one** active VK per layer in state at any instant,
mirroring the OL checkpoint predicate's single-slot model. We do *not* need to
store old and new simultaneously:

* The swap happens atomically at a **non-straddling proof boundary** (an OL
  epoch for the checkpoint VK; an EE batch for the EE VK). Because proofs are
  verified in order, the verifier checks the last old-logic proof, swaps, then
  checks the first new-logic proof — there is never an instant where two VKs are
  needed in state.
* **Historical** verification (fresh sync / replay) recovers the
  then-current VK by *replaying* the upgrade events in order, the same way ASM
  predicate swaps are reconstructed during replay. "One slot in state" does
  **not** mean
  "the old VK is never needed again"; it means it is derived by replay, not
  retained.

### P4. Two distinct kinds of change ride two distinct mechanisms

1. **Forks pre-baked into the active ELF** (height-conditional logic, e.g. an
   EVM hardfork the ELF already knows about). These fire **deterministically by
   height inside the ELF** with the *same VK* — no governance event at all. This
   is the true Ethereum-`chainspec`/`SpecId` model and is the cheapest upgrade
   path. A single proof may freely straddle such a fork because the ELF handles
   both rule-sets internally.
2. **Changes not anticipated in the active ELF** (new code, a fix that cannot be
   expressed as in-ELF conditional logic, a proof-system change). These require
   a new ELF → new VK → the authorized, L1-anchored rollover described here.

The design goal is to **maximize case 1** (pre-load known future fork heights
into the ELF at authorization time) so routine planned upgrades need no further
governance event, and reserve the heavyweight VK rollover for the rest.

### P5. Safety floor vs. liveness are different classes — do not conflate them

* The exit window is a **safety floor**. A lagging/stalling sequencer can only
  *delay* activation, which *lengthens* the window and therefore only helps
  exiting users. It can never compromise safety.
* "Enact a critical fix ASAP" is a **liveness/speed** desire. You *cannot* safely
  enact faster than the exit window without breaking the guarantee that protects
  users. Therefore the delayed-upgrade path is the **wrong tool** for an
  actively-exploited bug.

The apparent contradiction between "delay for exit" and "fix bugs fast" is a
category error: they are different classes, and the delayed-upgrade path should
not try to serve both.

Fast handling of critical bugs is **explicitly out of scope** for the
VK-upgrade mechanism. Two reasons make that acceptable:

* **The VK change is opaque.** A VK reveals nothing about *what* changed, so the
  upgrade path cannot make fine-grained "this is an urgent fix" decisions anyway
  — bug-driven urgency does not belong here.
* **The bridge trust model is the ultimate backstop.** Funds are held under the
  bridge's 1/N honesty assumption. If a flaw would otherwise misappropriate
  funds, an out-of-band correction can keep them from landing in the bad state.
  It may require major changes and is a heavy last resort, but from that angle
  the system is safe *without* a dedicated emergency-halt primitive in this
  design.

### P6. Sequencer-induced delay is the general liveness problem

"The sequencer can delay an upgrade by lagging on L1" is a special case of "the
sequencer can delay everything." Do not design a bespoke anti-stall just for VK
upgrades; upgrade-liveness rides on the general liveness backstop.

The backstop available today is **sequencer rotation**, not forced inclusion
(which is not yet implemented). Rotation also goes through L1 — so it is
observable and authority-gated — but, unlike a VK upgrade, it **applies
immediately**, with no enactment delay. The reason is a clean distinction:

> A delay is required only for changes that alter the **rules** users are
> trusting. Changing the **operator** under fixed rules touches no user's exit
> assumption, so it needs no exit window and can take effect at once.

This bounds sequencer-induced upgrade delay to roughly "one rotation-authority
coordination round + L1 settlement" — not anything on the 2016-block scale. The
residual cost is exactly that coordination lift (the rotation authority is a
separate multisig). Forced inclusion remains the longer-term, trust-minimized
version of the same backstop, and is also what an airtight L1-forced-*exit*
guarantee ultimately depends on.

### P7. The new code must be public at announcement

For the exit right to be *meaningful*, users must be able to inspect the new
logic to decide whether they object. An opaque VK gives them nothing to
evaluate. So authorization must publish the **source / ELF** (ideally via
reproducible builds), not merely the VK. Note that full nodes only need the VK
to *verify* (they do not run the ELF); only provers need the ELF binary.

## 3. Per-layer designs

### 3.1 ASM

**Mechanism.** Updates to the ASM VK take place entirely on the ASM client and
involve only Bitcoin. The update proposal transaction is sent to L1, and
activation occurs **exactly `2016` Bitcoin blocks after the update tx appears
on-chain** (one difficulty period, ≈ 2 weeks — a natural, L1-native exit
window).

**Why this is the easy case.** There is a single client and a single clock (the
Bitcoin chain). Authorization and the activation height are both L1-observable;
there is no second clock to reconcile (P2 is trivially satisfied because the
verifier *is* the L1 follower). Single-slot (P3) holds directly: the ASM swaps
its VK at the activation height and never needs two at once.

### 3.2 OL

**Where the parts live.** The OL VK update is authorized on the ASM client, but
it also involves the **OL sequencer** and **OL full nodes**, which are separate
clients from the ASM. This is where the two-clock hazard (P2) first appears.

**The problem with naive (pure-Bitcoin-block) activation.**

* *[major]* If activation is measured purely in Bitcoin blocks (as in the ASM
  case), the OL sequencer can seal a batch *before* the VK change is active —
  i.e. under the **old** logic — yet the corresponding checkpoint transaction
  lands on L1 *after* the activation height. A verifier keying off its own L1 tip
  then rejects that checkpoint as invalid, forcing the sequencer to **replay the
  whole batch**. This is exactly the two-clock mismatch of P2: the ASM (L1 tip)
  runs ahead of the checkpoint stream it verifies.
* *[minor]* The OL sequencer and full nodes may observe Bitcoin blocks at
  slightly different times and therefore activate at slightly different moments,
  causing OL sync divergence.

**Adopted approach: activate by the *L1 view within the checkpoint*.** Instead
of any node's local wall-clock view of Bitcoin, the decision point is the L1
view *committed in the checkpoint* (P2's shared coordinate):

1. When the OL sequencer reaches the **enactment block** (`update_block + 2016`),
   it **immediately seals the current batch with the L1 view of the *previous*
   block**. That sealed checkpoint is the **last** checkpoint under the old
   logic. (Whether the enactment block itself already carries the new VK is a
   matter of convention.)
2. Any block from that point onward — the first block of the new batch — runs
   the **new** logic.
3. **Reading rule for OL nodes:** if the (last block of the) L1 view in a
   checkpoint is *before* the enactment block, verify with the pre-update logic;
   if it is *at/after*, verify with the new logic. This is the same rule used for
   the ASM VK, now expressed against the checkpoint's committed L1 view rather
   than each node's local L1 tip.

Because every node reads the **same** L1 view from the **same** checkpoint, the
major replay hazard and the minor timing-divergence both disappear: activation
is a deterministic function of an artifact all nodes share, not of local
observation timing.

**Delay budget.** End to end, an OL upgrade stacks three delays:

* **Coordination** (multisig sign + L1 settlement) — front-loaded, *before* the
  window starts; magnitude unknown but small relative to the rest.
* **Enactment** — `2016` Bitcoin blocks ≈ 14 days. This *is* the exit window.
* **OL application tail** — because L1 information is applied only at the batch
  **terminal** block (a DA optimization), the swap lands batch-granularly: up to
  one batch (planned ≈ 9 h, ~2.7 % of the 14-day window) plus the OL's reorg lag.

The tail falls *after* enactment, so it only *lengthens* the effective exit
window (safe, P5). The OL reorg lag is subsumed: at 2016 blocks deep the
enactment boundary is far past any realistic reorg. Because the tail is a few
percent on a two-week floor, prefer the **relaxed** STR-3480 option (below) over
forcing an off-cadence short batch — the precise cut buys ≈ 9 h of tightness that
the exit direction does not need.

**Open question (STR-3480).** What if, for some reason, the L1 view is *not* cut
exactly on the enactment block? Options under consideration:

* Require the batch to be sealed with L1 view up to exactly the enactment block
  (the ticketed approach — precise, but may force an off-cadence short batch), or
* *(recommended)* Relax the condition and look at the **first** block of the L1
  view (the previous last-block-of-L1-view plus one, unless L1 has not advanced)
  to decide old-vs-new, letting the whole straddling batch run old logic.

Per the delay budget above, the relaxed option is preferred; the exact-cut
tradeoff is the remaining boundary-alignment detail to settle.

**Single slot is preserved.** Checkpoints are verified in epoch order; the ASM
verifies the last old-logic checkpoint, swaps the predicate, then verifies the
first new-logic checkpoint. One live VK at all times (P3); old VK recovered by
replay.

### 3.3 EE

**Where the parts live.** The EE VK update takes place on the **OL snark
account**, but it also involves the **EE sequencer** and **EE full nodes**, and
the update *procedure* is driven by the **ASM**. The main challenge is keeping
OL and EE in sync on *when* to activate (the same OL/EE-vs-ASM/OL clock problem
as §3.2) while also letting EE nodes that sync from Bitcoin verify that
activation happened at the correct time.

**Proposal: one extra snark update under the old VK ("wiggle room").** Once the
enactment target block is buried and processed by the OL (CSM) and observed by
Alpen/EE, allow a **single** snark update under the old logic/VK; every snark
update after that must use the new logic/VK.

**Why the wiggle room is needed.** We cannot guarantee the snark update lands on
exactly the correct L1 block:

* If we submit the snark update *one block before* the enactment target block
  (so EE switches logic and starts a new EE batch), we cannot know whether the
  new EE batch will be sealed before the next (enactment) L1 block.
* If we seal the batch *exactly* on the enactment block, proof-generation time
  may let a new L1 block arrive before the snark update is posted. A
  **commit → prove** approach (commit cheaply on time, supply the proof shortly
  after) mitigates this, but races against L1 blocks can still occur.

Allowing exactly one trailing old-VK update absorbs this timing jitter without
shrinking the exit window meaningfully (one extra update against a 2016-block
delay is negligible), and it keeps the activation rule implementable in the
presence of real proving latency.

**Key assumption.** EE nodes that sync from L1 (in real time, or a "retro" sync
from genesis) **do not** need to independently determine when the VK update
happened. They hold the OL VK, and by verifying the OL batches it is the **OL
program** that guarantees the update was made at the proper time.

* *If this assumption does not hold, this proposal must be revised.*
* For EE nodes that sync via both L2 and L1 (finalizing L2-synced blocks via
  L1), nothing is expected to change.

**Anti-stall constraint (optional).** To stop a malicious/lazy EE sequencer from
delaying the update, we can require the snark update to land at most, say, **3
L1 blocks** from the enactment block. Combined with a commit → prove procedure
(which decouples the deadline from proving time), 3 blocks is a safe margin.
This is judged a **high lift** and is not proposed for immediate implementation.

**Recommended variant (trust-minimized, against a malicious OL sequencer).**
Rather than relying on OL and EE to process L1 blocks at the same burial depth
(which requires EE to trust the OL sequencer's timing), have the EE
sequencer/nodes **wait until the OL submits a checkpoint whose committed L1 view
is past the enactment block**, and only then switch to the new logic — again
with the one-snark-update wiggle room. This makes the **L1 view in the
checkpoint** the decision point at the EE layer too, exactly as in §3.2, so the
same shared coordinate (P2) governs all three layers and EE never has to trust
the OL sequencer's local clock.

## 4. Alternatives considered and rejected

| Alternative | Verdict |
|-------------|---------|
| **Overwrite the VK as soon as the authorization is applied** | Rejected: activation timing non-deterministic, not announceable, no exit window, sequencer cannot coordinate. |
| **OL-derived activation** (e.g. "+D epochs after the log applies") | Rejected: absolute activation still hostage to L1/checkpoint timing; `D` is a magic constant; not announceable when authorized. |
| **Per-EE-block-height VK schedule** | Rejected: breaks the one-proof-one-VK invariant; block-height rule changes belong *inside* the ELF (P4 case 1), not in the VK schedule. |
| **"Bake everything into the ELF, never change the VK"** | Impossible as a complete solution: the VK is a function of the ELF, so a new fork height *is* a new VK. Useful only for forks pre-baked at authorization time (P4 case 1); cannot authorize a genuinely new prover program. |
| **Activate OL VK purely by Bitcoin block height** | Rejected: the major/minor two-clock problems of §3.2 — batch replay and node-timing divergence. |
| **EE tracks L1 burial independently to time activation** | Workable with wiggle room, but requires trusting the OL sequencer's timing / risks OL-EE divergence. Superseded by the checkpoint-L1-view variant (§3.3). |
| **One emergency mechanism that also does fast bug fixes via the upgrade path** | Rejected: category error (P5). Emergencies need a separate halt/pause primitive. |

## 5. Recommended way forward

1. **Use the L1 view committed in the proof as the universal activation
   coordinate** at every layer (P2). ASM: the Bitcoin height directly. OL: the
   L1 view in the checkpoint. EE: the L1 view in the OL checkpoint that crosses
   enactment (the trust-minimized variant of §3.3).
2. **Derive the activation point as `inclusion_height + 2016`** (P1): the payload
   carries only the new VK, and every node computes the boundary from the update
   tx's L1 inclusion height plus the fixed `2016`-block (one difficulty period)
   delay — measured forward from inclusion, so it is always known in advance and
   movable by neither the admin nor the sequencer.
3. **Keep one live VK slot per layer** (P3); reconstruct historical VKs by
   replay, mirroring the ASM-predicate single-slot model.
4. **Prefer pre-baking known forks into the ELF** (P4 case 1) so routine
   upgrades need no governance event; reserve VK rollover for unanticipated
   changes.
5. **Allow exactly one trailing old-VK snark update at the EE layer** to absorb
   proving-time jitter; layer a commit → prove flow under it if/when an anti-stall
   deadline is wanted.
6. **Leave fast critical-bug handling out of scope** (P5): the VK change is
   opaque, and the bridge's 1/N trust model plus out-of-band correction is the
   ultimate backstop. Do not route urgent fixes through the delayed-upgrade path.
7. **Use immediate sequencer rotation as today's liveness backstop** (P6):
   rotation goes through L1 but applies at once, because it changes the operator
   and not the rules. Forced inclusion is the longer-term version and is what an
   airtight L1-forced-exit guarantee ultimately depends on.
8. **Publish the new ELF/source at announcement** (P7).

## 6. Open questions

* **OL boundary alignment (STR-3480):** require the batch to be sealed with L1
  view exactly up to the enactment block, or relax to "first block of the L1
  view"? See §3.2.
* **EE activation convention:** do we *recommend* sealing the EE batch exactly on
  the enactment block, and do we add the ~3-L1-block anti-stall constraint plus a
  commit → prove flow? See §3.3 (currently deferred as high lift).
* **EE-from-L1 assumption:** confirm that EE nodes syncing from L1 can rely
  entirely on the OL program to attest activation timing (§3.3). If not, the EE
  proposal must be revised.
* **Activation denomination for OL/EE:** OL epoch (mechanism-clean, matches the
  checkpoint stream) vs. L1 height (matches the exit guarantee directly but needs
  straddle handling). The checkpoint-L1-view approach effectively chooses L1
  height as the coordinate while reading it from the proof.
* **Critical-bug handling:** out of scope (P5); backstopped by the bridge 1/N
  model plus out-of-band correction. Confirm that backstop is acceptable for the
  threat model.
* **Forced inclusion:** not yet implemented; the interim liveness backstop is
  immediate sequencer rotation (P6). Forced inclusion is the prerequisite for an
  airtight L1-forced-exit guarantee.

## 7. References

* STR-3480 — seal a batch with L1 view up to the enactment block.
* `crates/ol/stf/src/manifest_processing.rs` — ASM-log processing entry points
  (`process_block_manifests`, `process_epoch_terminal`,
  `process_ee_predicate_key_update`).
* `crates/ol/state-types/src/snark_account.rs` — `OLSnarkAccountState.update_vk`
  and `set_update_vk` (the EE account VK slot).
* `crates/ee-acct-runtime/src/verification_state.rs` — chunk-proof verification
  against the predicate key.
* `crates/proof-impl/alpen-acct/src/program.rs` — compile-time chunk predicate
  key baked into the acct guest.
* SPS-60 (Moho), SPS-62/63 (checkpoint), SPS-64 (bridge) — protocol context.
