# strata-prover-core

The core proving engine for Strata. Each prover instance handles one proof type
end-to-end: fetching inputs, generating proofs (locally or via a remote backend),
storing receipts, and retrying on failure.

## What problem does this solve?

Proof generation involves a lot of moving parts — input preparation, host selection,
error handling, retries, persistence, crash recovery. Without a shared engine, every
consumer (checkpoint prover, EE chunk prover, etc.) would re-implement all of that.

prover-core extracts the common lifecycle so consumers only define **what** to prove
and **how to get the input**. Everything else — scheduling, retry, storage, host
dispatch — is handled here.

## How it relates to zkaleido

[zkaleido](../zkaleido) owns the zkVM abstraction: programs, hosts, receipts.
prover-core never talks to a zkVM directly — it calls zkaleido's traits through
a pluggable strategy layer.

```
                      zkaleido land                    prover-core land
                 ┌───────────────────────┐      ┌──────────────────────────┐
 Consumer ──────▶│ ZkVmProgram::Input    │─────▶│ ProofSpec::fetch_input   │
                 └───────────────────────┘      └────────────┬─────────────┘
                                                             │
                 ┌───────────────────────┐      ┌────────────▼─────────────┐
                 │ ZkVmHost / RemoteHost │◀─────│ ProveStrategy::prove()   │
                 │ prove / start+poll    │      │ (NativeStrategy or       │
                 └───────────┬───────────┘      │  RemoteStrategy)         │
                             │                  └────────────┬─────────────┘
                 ┌───────────▼───────────┐      ┌────────────▼─────────────┐
                 │ ProofReceiptWithMeta   │─────▶│ ReceiptStore / Hook      │
                 └───────────────────────┘      └──────────────────────────┘
```

The key zkaleido types prover-core depends on:

- **`ZkVmProgram`** — defines a provable program (`Input`, `Output`, `prove()`).
  Consumed via `ProofSpec::Program`.
- **`ZkVmHost`** / **`ZkVmRemoteHost`** — local and remote proving backends.
  Captured inside strategy implementations at build time.
- **`ProofReceiptWithMetadata`** — the proof artifact that comes out the other end.

## Core concepts

### ProofSpec — the consumer's only job

A `ProofSpec` is the single trait consumers implement. It answers three questions:

1. What identifies a task? (`type Task`)
2. What program runs? (`type Program`)
3. How do you get the input? (`fn fetch_input`)

```rust
#[async_trait]
pub trait ProofSpec: Send + Sync + 'static {
    // Into<Vec<u8>> + TryFrom<Vec<u8>> for byte-key storage.
    // Must be deterministic (same task → same bytes).
    type Task: Clone + Debug + Display + Eq + Hash + Send + Sync
        + Into<Vec<u8>> + TryFrom<Vec<u8>> + 'static;
    type Program: ZkVmProgram<Input: Send + Sync> + Send + Sync + 'static;

    async fn fetch_input(
        &self,
        task: &Self::Task,
    ) -> ProverResult<<Self::Program as ZkVmProgram>::Input>;
}
```

A concrete example — proving OL checkpoints:

```rust
struct CheckpointSpec { storage: Arc<NodeStorage> }

#[async_trait]
impl ProofSpec for CheckpointSpec {
    type Task = Epoch;
    type Program = CheckpointProgram;

    async fn fetch_input(&self, epoch: &Epoch) -> ProverResult<CheckpointProverInput> {
        let header = self.storage.get_epoch_header(epoch)
            .map_err(|e| ProverError::TransientFailure(e.to_string()))?;
        let state = self.storage.get_state_at(epoch)
            .map_err(|e| ProverError::TransientFailure(e.to_string()))?;
        Ok(CheckpointProverInput { header, state })
    }
}
```

That's the entire integration surface. No storage wiring, no host selection, no
retry logic.

### ProveStrategy — how proving actually happens

The strategy is the bridge between prover-core and zkaleido's host layer. The host
type is captured at build time and erased, so `Prover<S>` has no host type parameter.

Two built-in strategies:

- **`NativeStrategy`** — calls `ZkVmProgram::prove()` directly. Good for tests, dev,
  and local RISC0.
- **`RemoteStrategy`** (behind the `remote` feature) — drives the async
  `start_proving` → poll `get_status` → `get_proof` cycle for backends like the
  SP1 network.

### Adding a new host (e.g. RISC0 remote, custom backend)

Adding a new proving backend doesn't require touching prover-core at all — it's
entirely a zkaleido concern. The steps:

1. **Implement `ZkVmHost`** in zkaleido for local execution, or `ZkVmRemoteHost`
   for an async remote backend. This is where the actual zkVM integration lives:
   input preparation, proof generation, status polling, receipt retrieval.
2. **Pass it to the builder** — `.native(your_host)` or `.remote(your_host)`.
   That's it. prover-core erases the host type behind a `ProveStrategy` and the
   rest of the system (specs, task lifecycle, PaaS) is completely unaware.

For example, a RISC0 remote prover would implement `ZkVmRemoteHost` with
`start_proving` submitting to Bonsai, `get_status` polling the Bonsai API, and
`get_proof` downloading the receipt. The consumer code and PaaS wiring stay identical
— only the `.remote(risc0_bonsai_host)` builder call changes.

If neither built-in strategy fits (e.g. a backend with a fundamentally different
execution model), you can implement `ProveStrategy<S>` directly and pass it to
`ProverBuilder::build()`.

### Task-based API — no UUIDs, no mapping

The entire API is keyed by your domain task type (`H::Task`), not opaque UUIDs.
You submit an `Epoch`, you wait for an `Epoch`, you get the receipt by `Epoch`.
No intermediate identifiers, no mapping tables, no string juggling.

```rust
// Submit and wait — you work with your own types, always
handle.submit(ChunkTask { batch_id, chunk_idx: 0 }).await?;
handle.submit(ChunkTask { batch_id, chunk_idx: 1 }).await?;

let tasks = vec![
    ChunkTask { batch_id, chunk_idx: 0 },
    ChunkTask { batch_id, chunk_idx: 1 },
];
let results = handle.wait_for_tasks(&tasks).await?;

// TaskResult gives you back your typed task
match &results[0] {
    TaskResult::Completed { task } => println!("done: {task}"),
    TaskResult::Failed { task, error } => println!("{task} failed: {error}"),
}

// Get receipt by task — no UUID lookup
let receipt = handle.get_receipt(&tasks[0])?;
```

This makes consumer code shorter and crash recovery simpler — the task store
is keyed by the serialized task, so after a restart the prover can deserialize
tasks directly from storage without an in-memory mapping table.

`ProofSpec::Task` requires `Into<Vec<u8>> + TryFrom<Vec<u8>>` for storage
serialization. The representation must be deterministic (borsh, bincode — not
serde_json).

### Task lifecycle

Every task moves through a simple state machine:

```
Pending → Proving → Completed
                 ↘ TransientFailure (retried → back to Proving)
                 ↘ PermanentFailure (terminal)
```

Retries are passive — `tick()` scans for retriable tasks and re-spawns them.
No background scheduler thread.

### Prover and ProverBuilder

You build a prover by combining a spec with a strategy and optional extensions:

```rust
let prover = ProverBuilder::new(spec)
    .receipt_store(sled_store)           // opt-in: receipt persistence by task
    .receipt_hook(checkpoint_db_hook)    // opt-in: domain-specific side-write
    .task_store(sled_task_store)         // default: InMemoryTaskStore
    .retry(RetryConfig::default())
    .native(host);                       // or .remote(host)
```

The consumer API is intentionally small:

| Method | What it does |
|--------|-------------|
| `submit(task)` | Spawn a background prove. Idempotent — submitting the same task twice is a no-op. |
| `execute(task)` | Submit + block until done. Returns `TaskResult<Task>`. |
| `wait_for_tasks(tasks)` | Block until all tasks reach a terminal state (watch-channel, zero-poll). |
| `get_receipt(task)` | Read the stored receipt (requires a configured `ReceiptStore`). |
| `get_status(task)` | Current `TaskStatus` for a task. |

## Optional extensions

### ReceiptStore

Persists proof receipts keyed by task (serialized to bytes). When configured,
prover-core auto-stores after proving and exposes `get_receipt(task)` on the handle.

`InMemoryReceiptStore` ships for tests. For production, implement against your DB.

### ReceiptHook

A typed callback that fires after a receipt is stored. Receives the full
`H::Task`, so it can write to domain-specific storage (e.g., a ProofDB keyed
by epoch). Most consumers don't need this — `ReceiptStore` + `get_receipt`
is usually enough.

## Task persistence

`TaskStore` handles task record persistence. Two implementations ship:

- **`InMemoryTaskStore`** — default, for tests and dev.
- **`SledTaskStore`** (behind the `sled` feature) — persistent, supports crash recovery.

Task records include an optional `metadata` field for strategy-specific state
(e.g., a remote `ProofId` for resuming polls after a restart).

## Feature flags

| Feature | What it enables |
|---------|----------------|
| `remote` | `RemoteStrategy` and `ProverBuilder::remote()`. Pulls in `zkaleido/remote-prover`. |
| `sled` | `SledTaskStore`. Pulls in `sled`. |

## What prover-core does

**Runs proofs.** Locally via `NativeStrategy` (any `ZkVmHost`) or remotely via
`RemoteStrategy` (`start_proving` → poll → `get_proof`, behind the `remote` feature).
The host type is erased at build time — consumer code never sees it.

**Manages task lifecycle.** `Pending → Proving → Completed / Failed`. Idempotent
submission (same task twice is a no-op), watch-channel notifications (zero-poll),
and typed `TaskResult<Task>` back to the caller.

**Retries on failure.** Transient errors get exponential backoff. `tick()` scans
for retriable tasks passively — no background scheduler threads.

**Survives crashes.** On restart, deserializes in-progress tasks directly from
their storage keys and re-spawns them. Remote proofs resume polling from the
saved `ProofId` instead of re-submitting — no double compute, no double cost.

**Persists state.** `TaskStore` (in-memory default, sled for production) tracks
task records. `ReceiptStore` (opt-in) persists proof receipts keyed by task.
`ReceiptHook` (opt-in) fires typed callbacks after receipt storage for
domain-specific side-writes.

### Planned

- **Send futures for remote proving** — `ZkVmRemoteProver` currently uses `?Send`
  futures, forcing one OS thread + runtime per remote proof. A one-line fix in
  zkaleido (`type Input: Send + Sync`) would allow all remote polls to share the
  main tokio runtime.
- **Metrics instrumentation** — counters for tasks submitted/completed/failed, histograms
  for proving duration. The hooks are natural extension points.

## What prover-core does NOT do

- **Service lifecycle** (start, stop, health) — that's PaaS.
- **Tick scheduling** — PaaS calls `tick()` on an interval.
- **Pipeline orchestration** (chunk → acct dependencies) — consumer code.
- **RPC exposure** — consumer's binary.
