# strata-prover-core

The core proving engine for Strata. Each prover instance handles one proof type
end-to-end: assembling inputs, generating proofs (locally or via a remote
backend), persisting receipts, and — the bulk of this crate — deciding *what to
do when something goes wrong*.

## What problem does this solve?

Proof generation has a lot of moving parts: input assembly, host selection,
receipt persistence, crash recovery, and — most subtly — failure handling.
Failures are not uniform. "The chunk I depend on isn't proven yet", "the gRPC
connection blipped", "the proving network expired my request", and "the guest
panicked" are four completely different situations that each want a different
response. Without a shared engine every consumer (checkpoint prover, EE chunk
prover, EE acct prover, …) would re-implement all of it, usually by collapsing
every failure into one "retry with backoff" hammer.

prover-core extracts the common lifecycle so consumers only declare **what** to
prove and **how to assemble the input**. Everything else — scheduling, the
retry/blocked/resubmit decision, backoff, persistence, host dispatch, crash
recovery — lives here, and treats each kind of failure on its own terms.

## How it relates to zkaleido

[zkaleido](../zkaleido) owns the zkVM abstraction: programs, hosts, receipts.
prover-core never talks to a zkVM directly — it calls zkaleido's traits through
a pluggable strategy layer.

```
                      zkaleido land                    prover-core land
                 ┌───────────────────────┐      ┌──────────────────────────┐
 Consumer ──────▶│ ZkVmProgram::Input    │─────▶│ ProofSpec::resolve_input │
                 └───────────────────────┘      └────────────┬─────────────┘
                                                             │ InputResolution
                 ┌───────────────────────┐      ┌────────────▼─────────────┐
                 │ ZkVmHost / RemoteHost │◀─────│ ProveStrategy::prove()   │
                 │ prove / start+poll    │      │ (Native or Remote)       │
                 └───────────┬───────────┘      └────────────┬─────────────┘
                             │ ZkVmError                      │ classified
                 ┌───────────▼───────────┐      ┌────────────▼─────────────┐
                 │ ProofReceiptWithMeta   │─────▶│ ReceiptStore / Hook      │
                 └───────────────────────┘      └──────────────────────────┘
```

The key zkaleido types prover-core depends on:

- **`ZkVmProgram`** — defines a provable program (`Input`, `Output`, `prove()`).
  Consumed via `ProofSpec::Program`.
- **`ZkVmHost`** / **`ZkVmRemoteHost`** — local and remote proving backends.
  Captured inside strategy implementations at build time.
- **`ZkVmError`** / **`RemoteProofFailureReason`** — the *typed* failure surface
  prover-core classifies into a retry decision (see [Failure classification](#failure-classification-the-heart-of-the-crate)).
- **`ProofReceiptWithMetadata`** — the proof artifact that comes out the other end.

## Core concepts

### ProofSpec — the consumer's only job

A `ProofSpec` is the single trait consumers implement. It answers three questions:

1. What identifies a task? (`type Task`)
2. What program runs? (`type Program`)
3. How do you assemble the input — and is it even ready? (`fn resolve_input`)

```rust
#[async_trait]
pub trait ProofSpec: Send + Sync + 'static {
    // `TaskKey` bundles these bounds: Clone + Debug + Display + Eq + Hash
    // + Into<Vec<u8>> + TryFrom<Vec<u8>> (byte-key storage) + Send + Sync + 'static.
    // Must be deterministic (same task → same bytes): borsh/bincode, not JSON.
    type Task: TaskKey;
    type Program: ZkVmProgram<Input: Send + Sync> + Send + Sync + 'static;

    async fn resolve_input(
        &self,
        task: &Self::Task,
    ) -> ProverResult<InputResolution<<Self::Program as ZkVmProgram>::Input>>;
}
```

The consumer speaks **domain facts**, not retry vocabulary. `resolve_input`
returns one of three outcomes, and the library decides scheduling:

```rust
pub enum InputResolution<I> {
    /// Input assembled — proceed to prove.
    Ready(I),
    /// A dependency isn't available yet. NOT a failure: the task parks in
    /// `Blocked` and is rechecked on a steady cadence, without burning the
    /// retry budget. `recheck_after` overrides the default cadence per task.
    Blocked { reason: String, recheck_after: Option<Duration> },
    /// This task can never produce valid input — terminal.
    Rejected { reason: String },
}
```

A concrete example — proving OL checkpoints:

```rust
struct CheckpointSpec { storage: Arc<NodeStorage> }

#[async_trait]
impl ProofSpec for CheckpointSpec {
    type Task = EpochCommitment;
    type Program = CheckpointProgram;

    async fn resolve_input(
        &self,
        epoch: &EpochCommitment,
    ) -> ProverResult<InputResolution<CheckpointProverInput>> {
        // `?` propagates an *infra* error (a DB read failed) — the library
        // retries it. Domain verdicts go through `InputResolution`, never `Err`.
        let Some(summary) = self.storage.epoch_summary(epoch)? else {
            // The epoch isn't finalized yet. Wait for it — this is not a failure.
            return Ok(InputResolution::Blocked {
                reason: format!("epoch {epoch} not finalized yet"),
                recheck_after: None,
            });
        };
        if summary.is_malformed() {
            return Ok(InputResolution::Rejected { reason: "corrupt summary".into() });
        }
        Ok(InputResolution::Ready(build_input(summary)))
    }
}
```

That's the entire integration surface. No storage wiring, no host selection, no
retry/backoff logic — and crucially, no confusing "is a missing dependency a
transient failure?" decision: it's `Blocked`.

> Migrating a spec that still classifies its own errors (returning
> `ProverError::transient(..)` / `permanent(..)`)? `InputResolution::from_result`
> bridges a legacy `Result<Input>` into a resolution: `Ok` → `Ready`, a permanent
> failure → `Rejected`, a transient one → `Blocked`, infra errors stay `Err`.

### Failure classification — the heart of the crate

A "failure" during proving is classified into exactly one **action**, decided
from the *typed* upstream error (not from which call-site produced it):

```rust
pub enum FailureAction {
    /// Retry, resuming any saved remote state (re-poll the same request).
    RetryResume,
    /// Retry after discarding saved remote state, so the next attempt submits a
    /// fresh request (the prior one expired / hit no capacity / was reverted).
    RetryFresh,
    /// Terminal — do not retry.
    Permanent,
}
```

`ProverError::Failed { action, msg }` carries that decision. The single source
of truth is the `classify` module, which maps `zkaleido::ZkVmError` and
`RemoteProofFailureReason` to an action:

| Upstream signal | Action | Why |
|---|---|---|
| `NetworkRetryableError`, `ProofNotReady` | `RetryResume` | transient transport / not ready yet |
| `ProofGenerationError`, `InvalidELF`, `InvalidInput`, `InvalidVerifyingKey` | `Permanent` | the submission/env is bad; resubmitting won't help |
| `InvalidProofReceipt`, `ProofVerificationError`, `OutputExtractionError`, `ExecutionError` | `Permanent` | the proof we got is broken, or the guest faulted |
| status `Unexecutable` | `Permanent` | guest can't run |
| status `Unfulfillable` / `Expired` / `Reverted` | `RetryFresh` | request is dead but the input is fine — resubmit |
| `ZkVmError::Other` (uncategorized) | `RetryResume` | conservative: retry, bounded by the task budget |
| status `Other` (uncategorized) | `Permanent` | conservative: an unclassifiable remote failure is treated as terminal |

The two `Other` catch-alls default in opposite directions on purpose: an
uncategorized zkVM error is more likely a transient blip (retry, budget-bounded),
whereas an uncategorized *remote request* failure is treated as terminal rather
than resubmitting an expensive proof blindly.

Strategy code reports *what happened*; this module decides *what to do*. This
fixes a class of bugs where the decision was made positionally — e.g. a 503 on
artifact download being marked permanent (and the finished proof abandoned), or
a genuinely fatal `ProofGenerationError` being retried for the whole budget.

### Two-tier retry

Not every hiccup deserves the heavyweight, persistent, tick-driven machinery.
Retries happen at two tiers:

```
   task-level tier        ┌─────────────────────────────────────────────┐
   (cross-attempt,        │ persistent state machine (TaskStore)         │
    survives crashes,     │ Pending → Proving → Completed                │
    tick-driven)          │            ↘ Blocked / TransientFailure       │
                          │ handles: dependency waits, resubmit, recovery │
                          └───────────────▲─────────────────────────────┘
                                          │ escalate only if persistent
   in-attempt tier        ┌───────────────┴─────────────────────────────┐
   (in-process, fast,     │ short local retry around idempotent polls    │
    invisible to the SM)  │ get_status / get_proof — few attempts, jitter │
                          └──────────────────────────────────────────────┘
```

- **In-attempt (`LocalRetryConfig`)** — `RemoteStrategy` wraps the idempotent
  remote polls (`get_status`, `get_proof`) in a short in-process retry on
  `RetryResume` errors. This recovers blips the backend itself gives up on fast
  (notably SP1 marking "Service was not ready: transport error" *permanent*)
  without tearing down the whole attempt to the slow task tier.
- **Task-level** — only sees failures that survived the local tier, plus the
  things that genuinely need persistence: dependency waits (`Blocked`), resubmit
  (`RetryFresh`), and crash recovery. Exponential backoff with jitter.

### Jitter, and differentiated budgets

- **Jitter** (`jitter_frac`, default ±20%) spreads the wake-ups of tasks that
  failed on the same tick, so they don't retry in a synchronized stampede
  against a shared backend. The offset is deterministic per task (FNV-1a over
  the key), so it's reproducible without an RNG.
- **Separate budgets**: `max_retries` (resume-class, default 15) vs
  `max_resubmits` (resubmit-class, default 3). A resubmit re-runs the *whole*
  proof, so it gets a much smaller budget than a cheap re-poll.

### Resume vs. resubmit

Remote proofs persist their `ProofId` (in the task record's `metadata`). On a
`RetryResume` the next attempt resumes polling that same id — no double
submission, no double cost. On a `RetryFresh` the saved id is cleared first
(`TaskStore::clear_metadata`), so the next attempt submits a brand-new request
— used when the prior request is dead (expired/unfulfillable/reverted).

### Stage checkpointing

The pipeline is `resolve_input → prove → receipt_store.put → receipt_hook →
Completed`. If proving already succeeded on a prior attempt, the receipt is in
the receipt store; on the next run prover-core skips `resolve_input` and the
(expensive) prove entirely and re-runs only the hook. A transient receipt-hook
failure therefore never re-proves. (Requires a configured `ReceiptStore`.)

### Task lifecycle

```
Pending ─▶ Proving ─▶ Completed                       (terminal)
   ▲          │  ╲
   │          │   ╲─▶ Blocked ──────────┐  (waiting on a dependency; rechecked,
   │          │        ▲                │   does NOT consume the retry budget)
   │          │        └────────────────┘
   │          ├─▶ TransientFailure ──────┐ (retry/resubmit after backoff)
   └──────────┴────────────────◀─────────┘
              ╲─▶ PermanentFailure                    (terminal)
```

`Proving` and `TransientFailure` carry both a `retry_count` and a
`resubmit_count`, so the budgets survive a mid-attempt crash. Re-runs are
passive: PaaS calls `tick()` on an interval, which scans for tasks whose
recheck/backoff time has elapsed (`TaskStatus::wants_rescan` — transient
failures *and* blocked tasks) and re-spawns them. No background scheduler thread.

### Crash recovery

On the first tick after startup, `recover()` re-spawns every unfinished
(`Pending`/`Proving`) task. A task found mid-`Proving` is counted as a synthetic
transient failure (bumping `retry_count`) so a crash loop is bounded by
`max_retries`. Blocked tasks are picked up by the normal recheck scan.

### Prover and ProverBuilder

You build a prover by combining a spec with a strategy and optional extensions:

```rust
let prover = ProverBuilder::new(spec)
    .receipt_store(sled_store)           // opt-in: receipt persistence (+ checkpointing)
    .receipt_hook(checkpoint_db_hook)    // opt-in: domain-specific side-write
    .task_store(sled_task_store)         // default: InMemoryTaskStore
    .retry(RetryConfig::default())
    .native(host);                       // or .remote(host) / .remote_with_interval(host, dur)
```

The consumer API is intentionally small:

| Method | What it does |
|--------|-------------|
| `submit(task)` | Spawn a background prove. Idempotent — submitting the same task twice is a no-op. |
| `execute(task)` | Submit + block until terminal. Returns `TaskResult<Task>`. |
| `wait_for_tasks(tasks)` | Block until all tasks reach a terminal state (watch-channel, zero-poll). Blocked is *not* terminal — waiters keep waiting. |
| `get_receipt(task)` | Read the stored receipt (requires a configured `ReceiptStore`). |
| `get_status(task)` | Current `TaskStatus` for a task. |

### RetryConfig knobs

```rust
RetryConfig {
    max_retries: 15,            // resume-class budget (cheap re-polls)
    base_delay_secs: 5,         // exponential backoff base …
    multiplier: 1.5,            //   … grows ×1.5 per attempt …
    max_delay_secs: 3600,       //   … capped at 1h
    jitter_frac: 0.2,           // ±20% randomized spread on each delay
    max_resubmits: 3,           // resubmit-class budget (each re-runs the proof)
    blocked_recheck_secs: 10,   // steady recheck cadence for Blocked tasks
    local: LocalRetryConfig {   // in-attempt tier for idempotent remote polls
        max_attempts: 5,
        base_delay_ms: 500,
        max_delay_ms: 10_000,
    },
}
```

Every field has a default that reproduces sensible behavior; `RetryConfig::default()`
is the drop-in starting point.

## How proving actually happens

Internally the prover bridges zkaleido's host layer through a crate-private
`ProveStrategy`. The host type is captured at build time and erased, so
`Prover<S>` has no host type parameter. Two strategies ship:

- **native** — calls `ZkVmProgram::prove()` directly. Good for tests, dev, and
  local proving.
- **remote** (behind the `remote` feature) — drives the async `start_proving` →
  poll `get_status` → `get_proof` cycle for backends like the SP1 network. The
  remote futures are `Send` (zkaleido `v0.3.0-rc.1` makes `start_proving` return
  `-> impl Future + Send`, with `ProofId` and `ZkVmProgram::Input: Send + Sync`),
  so each `prove()` drives them directly on the strategy's runtime via
  `Runtime::block_on` — no `LocalSet` thread-pinning. That runtime is still a
  **single long-lived multi-thread runtime** owned by the strategy (built once,
  not per-call), and that longevity matters: SP1 SDK ≥6.2 caches its gRPC channel
  process-wide, and a per-call runtime would kill that channel's background
  worker after the first prove — every later call then failing with "Service was
  not ready: transport error".

### Adding a new host (e.g. RISC0 remote, custom backend)

Adding a proving backend doesn't touch prover-core — it's a zkaleido concern:

1. **Implement `ZkVmHost`** (local) or `ZkVmRemoteHost` (async remote) in
   zkaleido. This is where the real zkVM integration lives: input prep, proof
   generation, status polling, receipt retrieval. Map your backend's failures
   onto `ZkVmError` variants thoughtfully — that's what drives the retry
   decision (see [Failure classification](#failure-classification-the-heart-of-the-crate)).
2. **Pass it to the builder** — `.native(your_host)` or `.remote(your_host)`.

A RISC0 remote prover would implement `ZkVmRemoteHost` with `start_proving`
submitting to Bonsai, `get_status` polling the Bonsai API, and `get_proof`
downloading the receipt. Consumer code and PaaS wiring stay identical — only the
`.remote(risc0_bonsai_host)` builder call changes.

## Optional extensions

### ReceiptStore

Persists proof receipts keyed by task (serialized to bytes). When configured,
prover-core auto-stores after proving, exposes `get_receipt(task)` on the handle,
and enables [stage checkpointing](#stage-checkpointing). `InMemoryReceiptStore`
ships for tests; implement against your DB for production.

### ReceiptHook

A typed callback that fires after a receipt is stored. Receives the full
`H::Task`, so it can write to domain-specific storage (e.g. a ProofDB keyed by
epoch). Most consumers don't need this — `ReceiptStore` + `get_receipt` is
usually enough.

### TaskStore

Persists task records. `InMemoryTaskStore` ships for tests; the node's storage
managers implement the trait against sled for production. Records are
borsh-friendly byte-keyed values and carry an optional `metadata` field for
strategy state (the remote `ProofId`); `clear_metadata` drops it for the
resubmit path.

## Feature flags

| Feature | What it enables |
|---------|----------------|
| `remote` | `RemoteStrategy`, `ProverBuilder::remote()` / `remote_with_interval()`, and the in-attempt retry tier. Pulls in `zkaleido/remote-prover`. |

## What prover-core does

- **Runs proofs** locally (`NativeStrategy`) or remotely (`RemoteStrategy`), host
  type erased at build time.
- **Classifies failures** from the typed error into resume / resubmit / permanent
  — one decision, one place — and distinguishes "blocked on a dependency" (not a
  failure at all) from "failed".
- **Retries in two tiers**: fast in-process retries for idempotent polls, durable
  tick-driven retries with jittered backoff and separate resume/resubmit budgets
  for everything else.
- **Doesn't re-prove needlessly**: resumes remote polls from the saved `ProofId`,
  and checkpoints the receipt so post-prove failures skip straight to the hook.
- **Survives crashes**: re-spawns unfinished tasks on restart from their storage
  keys; remote proofs resume their existing request.
- **Persists state** via `TaskStore` / `ReceiptStore` / `ReceiptHook` (in-memory
  defaults, consumer-provided for production).

### Planned

- **Distinct `AwaitingResubmit` state** — currently resubmit reuses
  `TransientFailure` with a cleared `ProofId`; a dedicated state would sharpen
  observability.
- **Metrics instrumentation** — counters per `(proof_type, action)`, histograms
  of attempts-to-success and proving duration.

## What prover-core does NOT do

- **Service lifecycle** (start, stop, health) — that's PaaS.
- **Tick scheduling** — PaaS calls `tick()` on an interval.
- **Pipeline orchestration** (chunk → acct dependencies) — consumer code (and the
  `Blocked` state, which lets a downstream task wait on an upstream one cleanly).
- **RPC exposure** — the consumer's binary.
</content>
