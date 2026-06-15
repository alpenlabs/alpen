# strata-paas

Prover-as-a-Service. Turns a `Prover<S>` from
[prover-core](../prover-core/README.md) into a managed service with command
channels, periodic retry/recheck ticks, and health monitoring.

## Why does this exist?

prover-core is a library — it knows how to prove things and how to handle every
flavor of failure, but it has no runtime, no event loop, and no concept of a
long-running service. Something has to own the lifecycle: receive commands, drive
the periodic `tick()` that fires retries, dependency rechecks, and crash
recovery, and hand callers a clean async handle.

That's PaaS. A thin service wrapper that bridges prover-core into the Service
Framework (SF), so provers launch, monitor, and shut down alongside the rest of
the node's services.

## How it fits together

```
prover-core                              paas
┌──────────────────────────┐            ┌──────────────────────────────┐
│ ProofSpec (resolve_input) │            │ ProverServiceBuilder          │
│ Prover<S>                │───────────▶│ ProverHandle<S>               │
│ ProverBuilder             │            │ SF service (command / ticking)│
│ ProveStrategy (nat/rem)   │            │  └─ tick() ⇒ retries,         │
│ retry/blocked/checkpoint  │            │     rechecks, recovery        │
│ TaskStore, ReceiptStore   │            └──────────────────────────────┘
└──────────────────────────┘
 Knows: proving, the retry/blocked/      Knows: SF lifecycle, command
 resubmit decision, recovery             routing, tick scheduling
```

prover-core does all the real work — including all retry, blocked-dependency, and
checkpoint logic. PaaS just gives it a place to live and a clock to tick on.

## Getting started

### Building a service

```rust
let prover = ProverBuilder::new(spec)
    .receipt_store(sled_store)
    .retry(RetryConfig::default())
    .native(host);

let handle = ProverServiceBuilder::new(prover)
    .tick_interval(Duration::from_secs(5))
    .launch(&executor)
    .await?;
```

The tick interval controls how often PaaS calls `prover.tick()`. The tick is the
heartbeat for everything time-driven in prover-core: re-spawning tasks whose
backoff has elapsed (`TransientFailure`), rechecking tasks waiting on a
dependency (`Blocked`), and one-shot startup crash recovery. If you don't set
one, the service runs in command-only mode — no retries, no rechecks, no
recovery; good for one-shot provers in tests. (Launching with a retry config but
no tick interval is rejected — retries would never fire.)

`RetryConfig` carries the full retry policy — resume/resubmit budgets, jittered
backoff, the blocked-recheck cadence, and the in-attempt local-retry budget.
See [prover-core's README](../prover-core/README.md#retryconfig-knobs) for the
knobs; PaaS just hands the config to the prover and calls `tick()`.

### Using the handle

`ProverHandle<S>` is what consumers hold. It's generic over the spec only — the
zkVM host type is already erased inside the prover. The entire API is keyed by
your domain task type: no UUIDs, no intermediate identifiers.

```rust
// Sequential — prove one thing and wait
let result = handle.execute(epoch).await?;
match result {
    TaskResult::Completed { task } => println!("proved epoch {task}"),
    TaskResult::Failed { task, error } => println!("{task} failed: {error}"),
}

// Fan-out — submit many, wait for all, typed results back
let chunks: Vec<_> = (0..n).map(|i| ChunkTask { batch_id, chunk_idx: i }).collect();
for c in &chunks {
    handle.submit(c.clone()).await?;   // idempotent — double-submit is a no-op
}
handle.wait_for_tasks(&chunks).await?;

// Get receipt by task — no UUID lookup
let receipt = handle.get_receipt(&chunks[0])?;
```

The full API:

| Method | Description |
|--------|-------------|
| `submit(task)` | Spawn a background prove. Idempotent. |
| `execute(task)` | Submit + block until terminal. Returns `TaskResult<Task>`. |
| `wait_for_tasks(tasks)` | Block until all reach a terminal state. Watch-channel, zero-poll. A `Blocked` task is *not* terminal — the wait stays parked. |
| `get_receipt(task)` | Read the stored receipt (requires a configured `ReceiptStore`). |
| `get_status(task)` | Current `TaskStatus` (`Pending`/`Proving`/`Blocked`/`Completed`/`TransientFailure`/`PermanentFailure`). |

`submit` and `execute` go through the SF command channel. `wait_for_tasks`,
`get_receipt`, and `get_status` read directly from shared prover state — no
channel round-trip.

## Real-world examples

### OL checkpoint prover

Sequential, one epoch at a time. A `ReceiptHook` side-writes proofs into the
domain's ProofDB. `resolve_input` returns `Blocked` while an epoch isn't yet
finalized, so the task waits instead of churning through the retry budget.

```rust
let prover = ProverBuilder::new(CheckpointSpec { storage })
    .receipt_store(sled_receipt_store)
    .receipt_hook(CheckpointDbHook { proof_db })
    .retry(RetryConfig::default())
    .native(CheckpointProgram::native_host());

let handle = ProverServiceBuilder::new(prover)
    .tick_interval(Duration::from_secs(10))
    .launch(&executor).await?;

handle.execute(epoch).await?;
```

### EE chunk/acct pipeline

Fan-out chunks, then aggregate into an acct proof. The shared receipt store is
the glue: the acct spec reads chunk receipts during `resolve_input`. When a chunk
receipt isn't present yet, the acct spec returns `InputResolution::Blocked` —
so the acct task **parks and rechecks on a steady cadence**, instead of faking a
`TransientFailure` and riding exponential-backoff-to-an-hour. Dependency-waiting
stops polluting failure metrics.

```rust
let receipt_store = Arc::new(SledReceiptStore::new(db));

// Chunk prover writes receipts.
let chunk_prover = ProverBuilder::new(ChunkSpec { block_storage })
    .receipt_store(receipt_store.clone())
    .retry(RetryConfig::default())
    .native(EeChunkProgram::native_host());
let chunk_handle = ProverServiceBuilder::new(chunk_prover)
    .tick_interval(Duration::from_secs(5)).launch(&executor).await?;

// Acct prover reads chunk receipts in resolve_input; missing ones ⇒ Blocked.
let acct_prover = ProverBuilder::new(AcctSpec {
        batch_storage,
        chunk_receipts: receipt_store.clone(),
    })
    .receipt_hook(AcctProofDbHook { proof_db })
    .retry(RetryConfig::default())
    .native(EeAcctProgram::native_host());
let acct_handle = ProverServiceBuilder::new(acct_prover)
    .tick_interval(Duration::from_secs(5)).launch(&executor).await?;

// Submit the whole batch up front. Chunks prove in parallel; the acct task
// blocks on missing chunk receipts and unblocks itself as they land.
for c in &chunks { chunk_handle.submit(c.clone()).await?; }
acct_handle.submit(AcctTask { batch_id }).await?;
```

### Switching to remote proving

The spec stays identical. Only the builder call changes:

```rust
let prover = ProverBuilder::new(spec)
    .receipt_store(sled_store)
    .task_store(persistent_task_store)   // so remote ProofIds survive restarts
    .retry(RetryConfig::default())
    .remote(sp1_host);                   // instead of .native(host)
```

Requires the `remote` feature on prover-core. The in-attempt local-retry tier
and the long-lived runtime that hosts the SP1 gRPC channel are both internal to
the remote strategy — no PaaS-side change.

## Feature status

### Implemented

- **Service Framework integration** — two modes:
  - *Command-only* — no ticking. Good for one-shot provers and tests.
  - *Ticking* — commands + periodic `tick()` for retry/recheck scanning and crash
    recovery.
- **Command routing** — `Submit` and `Execute` flow through a typed async channel;
  completion senders return results to the caller.
- **ProverHandle** — cloneable, generic over spec only. Channel-based for
  commands, direct-read for queries (no round-trip).
- **Tick scheduling** — configurable interval; drives retries, dependency
  rechecks, and recovery without background threads.
- **Service status** — task count via `get_status()`, on both modes.

### Planned

- **Health check API** — the `ServiceMonitor` is held internally but not exposed
  on the handle; a `health()` method would let orchestrators check liveness
  without the command channel.
- **Graceful shutdown** — coordinated drain of in-flight tasks before teardown,
  so remote proofs aren't abandoned mid-poll.
- **RPC bridge** — a thin adapter exposing `submit`/`execute`/`get_receipt` over
  JSON-RPC or gRPC for external tooling.

## What PaaS does NOT do

- **Proving** — prover-core's strategy layer.
- **The retry / blocked / resubmit / checkpoint decisions** — all prover-core.
  PaaS only provides the clock (`tick`) that fires them.
- **Pipeline orchestration** — consumer code decides what to submit; the `Blocked`
  state handles cross-task dependency waits.
- **RPC** — the binary crate's concern.
</content>
