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

For the choice to exit to be *meaningful*, the new **code** — not just the VK —
must be published at announcement: a VK is opaque, so users need the source
(ideally via reproducible builds) to decide whether to object. Full nodes need
only the VK to verify; only provers need the ELF.

## 2. Design principles

The entire design follows from a single question — **what clock measures the
activation point?** — so this section is really one principle (anchor on L1) and
its direct consequences.

### Anchor activation on L1, not on a sequencer-paced height

The activation point needs a clock with two properties: **users can watch it**
(they decide whether to exit by seeing when the switch will happen), and **the
sequencer cannot control it** (otherwise it can move the deadline). Only
Bitcoin / L1 has both — which rules out the option that would have been far
simpler.

**Why not an L2 height?** Scheduling activation at an L2 block height ("at OL/EE
block `N`, switch the VK") is by far the easiest to build: each STF already
tracks its own height, so activation is a local `if height >= N` check — the
Ethereum fork-by-height model, with none of the L1 plumbing the rest of this
document needs. But the sequencer sets the pace of L2 blocks, so it decides when
that height is reached. That breaks two things:

* **The exit window can be shrunk.** With activation at L2 block 1,500,000, the
  sequencer can mint blocks fast and arrive in an hour instead of two weeks. A
  *shorter* window is a safety failure, not an inconvenience.
* **The schedule can be overshot.** The sequencer can get a checkpoint past block
  1,500,000 finalized under the old VK; the swap then cannot be applied without
  rejecting a finalized checkpoint, because nothing the checkpoint commits to
  makes the boundary decidable.

(And "block `N`, in `T` days" is not a deadline users can rely on when the
sequencer sets the rate.)

**Why L1 works.** Bitcoin advances on its own at ≈ 10 min/block, so define

```
B = (L1 inclusion height of the update tx) + 2016     (≈ 2 weeks)
```

`B` is a fixed wall-clock deadline the sequencer **cannot compress**, every node
computes it identically, and users can watch it. The sequencer can still *lag*
(cross `B` late), but lag only *lengthens* the window — which is safe.

**Nothing extra goes in the payload.** `B` is fully determined the moment the
update tx lands (`inclusion + 2016`), so the payload carries only the new VK — an
explicit activation height would be redundant. This is the
authorization-vs-activation split made concrete: the L1 transaction is the
**authorization** ("what"), and `B = inclusion + 2016` is the **activation**
("when"), set by neither the admin nor the sequencer.

How each layer reads `B` from its proofs — and why it must use the L1 view
*committed in the proof* rather than its own L1 tip — is the "two-clock"
subtlety worked through in §3.2.

### Enforcing activation: safety vs. liveness

The sequencer paces the proof stream, so it influences *when* the system crosses
`B`. That influence splits into two questions with opposite answers.

**Safety — no early switch, no wrong-VK acceptance — is fully enforced, with no
trust between nodes.** The reading rule:

1. Every node derives `B = inclusion_height + 2016` independently from L1.
2. Every proof commits its L1 view `V`, checked against the real L1 chain, so `V`
   cannot be forged.
3. Verify under the new VK iff `V` crosses `B`, else the old VK; reject anything
   that does not verify under the rule-selected key.

This cuts **both** ways: a post-`B` proof under old logic is rejected (no late
switch), and a pre-`B` proof under new logic is rejected (no *early* switch —
which is what actually enforces the exit window). `B` is an L1 fact every node
computes identically, so enforcement is decentralized.

**Liveness — forcing the switch to happen — is not intrinsically enforceable,
but the gap is narrow.** No rule can make a stalled sequencer *produce* a post-`B`
proof. But it **cannot validly process new L1 content under old logic** either:
past `B`, its only choices are (a) switch to the new logic, or (b) freeze its L1
view — which stops crediting deposits, withdrawals, and all L1-originated
activity. There is no "keep running normally on old logic" option, so any delay
shows up as a **visible halt**.

**A halt is handled by sequencer rotation.** Rotation also goes through L1 but
**applies immediately**, with no exit window — because a window is needed only to
change the **rules** users trust, and rotation changes the **operator**, not the
rules. This bounds sequencer-induced delay to one rotation-authority coordination
round plus L1 settlement (the cost being that coordination — rotation is a
separate multisig). Forced inclusion is the longer-term, trust-minimized version,
and is what an airtight L1-forced-exit guarantee ultimately needs; it is not yet
implemented.

**Critical bugs are deliberately out of scope.** You cannot enact faster than the
exit window without breaking the guarantee, so this path is the wrong tool for an
actively-exploited bug. That is acceptable because a VK change is **opaque** (it
says nothing about *what* changed, so the path cannot make "this is urgent"
decisions anyway), and funds sit under the bridge's **1/N** assumption, where an
out-of-band correction (heavy, possibly large changes) can keep a flaw from
misappropriating them. No dedicated emergency-halt primitive is specified here.

**Optional hard rule.** A deadline ("if no proof with `V ≥ B` lands within `N` L1
blocks of `B`, reject further old-logic proofs") makes the halt a consensus rule.
It still cannot force activation — only force the stall into an explicit,
rotation-triggering halt. Since freezing is already self-evident, this may not be
worth the lift.

**Open item — EE-from-L1.** For ASM and OL the verifier has direct L1 access and
enforces `B` itself. For EE, whether a node syncing from L1 checks activation
timing itself or delegates it to the OL proof is unsettled (§3.3); feasibility
holds either way, only the *location* of the check changes.

### One live VK per layer

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

## 3. Per-layer designs

### 3.1 ASM

**Mechanism.** Updates to the ASM VK take place entirely on the ASM client and
involve only Bitcoin. The update proposal transaction is sent to L1, and
activation occurs **exactly `2016` Bitcoin blocks after the update tx appears
on-chain** (one difficulty period, ≈ 2 weeks — a natural, L1-native exit
window).

**Why this is the easy case.** There is a single client and a single clock (the
Bitcoin chain). Authorization and the activation height are both L1-observable;
there is no second clock to reconcile, because the verifier *is* the L1
follower. The single-VK slot holds directly: the ASM swaps
its VK at the activation height and never needs two at once.

### 3.2 OL

**Where the parts live.** The OL VK update is authorized on the ASM client, but
it also involves the **OL sequencer** and **OL full nodes**, which are separate
clients from the ASM. This is where the two-clock hazard first appears.

**The problem with naive (pure-Bitcoin-block) activation.**

* *[major]* If activation is measured purely in Bitcoin blocks (as in the ASM
  case), the OL sequencer can seal a batch *before* the VK change is active —
  i.e. under the **old** logic — yet the corresponding checkpoint transaction
  lands on L1 *after* the activation height. A verifier keying off its own L1 tip
  then rejects that checkpoint as invalid, forcing the sequencer to **replay the
  whole batch**. This is exactly the two-clock mismatch: the ASM (L1 tip)
  runs ahead of the checkpoint stream it verifies.
* *[minor]* The OL sequencer and full nodes may observe Bitcoin blocks at
  slightly different times and therefore activate at slightly different moments,
  causing OL sync divergence.

**Adopted approach: activate by the *L1 view within the checkpoint*.** Instead
of any node's local wall-clock view of Bitcoin, the decision point is the L1
view *committed in the checkpoint* (the shared coordinate):

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
window (safe). The OL reorg lag is subsumed: at 2016 blocks deep the
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
first new-logic checkpoint. One live VK at all times; old VK recovered by
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
same shared coordinate governs all three layers and EE never has to trust
the OL sequencer's local clock.

## 4. Alternatives considered and rejected

| Alternative | Verdict |
|-------------|---------|
| **Overwrite the VK as soon as the authorization is applied** | Rejected: activation timing non-deterministic, not announceable, no exit window, sequencer cannot coordinate. |
| **OL-derived activation** (e.g. "+D epochs after the log applies") | Rejected: absolute activation still hostage to L1/checkpoint timing; `D` is a magic constant; not announceable when authorized. |
| **Per-EE-block-height VK schedule** | Rejected: breaks the one-proof-one-VK invariant; block-height rule changes belong *inside* the ELF (height-conditional logic in the guest), not in the VK schedule. |
| **"Bake everything into the ELF, never change the VK"** | Impossible as a complete solution: the VK is a function of the ELF, so a new fork height *is* a new VK. Useful only for forks pre-baked into the ELF at authorization time; cannot authorize a genuinely new prover program. |
| **Activate OL VK purely by Bitcoin block height** | Rejected: the major/minor two-clock problems of §3.2 — batch replay and node-timing divergence. |
| **EE tracks L1 burial independently to time activation** | Workable with wiggle room, but requires trusting the OL sequencer's timing / risks OL-EE divergence. Superseded by the checkpoint-L1-view variant (§3.3). |
| **One emergency mechanism that also does fast bug fixes via the upgrade path** | Rejected: category error — the exit delay and fast bug-fixing are different classes. Critical bugs are out of scope (bridge 1/N backstop), not a VK-upgrade concern. |

## 5. Recommended way forward

1. **Use the L1 view committed in the proof as the universal activation
   coordinate** at every layer. ASM: the Bitcoin height directly. OL: the
   L1 view in the checkpoint. EE: the L1 view in the OL checkpoint that crosses
   enactment (the trust-minimized variant of §3.3).
2. **Derive the activation point as `inclusion_height + 2016`:** the payload
   carries only the new VK, and every node computes the boundary from the update
   tx's L1 inclusion height plus the fixed `2016`-block (one difficulty period)
   delay — measured forward from inclusion, so it is always known in advance and
   movable by neither the admin nor the sequencer.
3. **Keep one live VK slot per layer**; reconstruct historical VKs by
   replay, mirroring the ASM-predicate single-slot model.
4. **Prefer pre-baking known forks into the ELF** (height-conditional logic, same
   VK) so routine
   upgrades need no governance event; reserve VK rollover for unanticipated
   changes.
5. **Allow exactly one trailing old-VK snark update at the EE layer** to absorb
   proving-time jitter; layer a commit → prove flow under it if/when an anti-stall
   deadline is wanted.
6. **Leave fast critical-bug handling out of scope:** the VK change is
   opaque, and the bridge's 1/N trust model plus out-of-band correction is the
   ultimate backstop. Do not route urgent fixes through the delayed-upgrade path.
7. **Use immediate sequencer rotation as today's liveness backstop:**
   rotation goes through L1 but applies at once, because it changes the operator
   and not the rules. Forced inclusion is the longer-term version and is what an
   airtight L1-forced-exit guarantee ultimately depends on.
8. **Publish the new ELF/source at announcement.**

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
* **Critical-bug handling:** out of scope; backstopped by the bridge 1/N
  model plus out-of-band correction. Confirm that backstop is acceptable for the
  threat model.
* **Forced inclusion:** not yet implemented; the interim liveness backstop is
  immediate sequencer rotation. Forced inclusion is the prerequisite for an
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
