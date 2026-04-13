# strata-paas

Prover-as-a-Service. Turns a `Prover<S>` from
[prover-core](../prover-core/README.md) into a managed service with command
channels, periodic retries, and health monitoring.

## Why does this exist?

prover-core is a library — it knows how to prove things, but it has no runtime,
no event loop, and no concept of a long-running service. If you want to run a
prover inside an application that accepts work over time, something needs to own
the lifecycle: receiving commands, driving periodic maintenance (retries, crash
recovery), and giving callers a clean async handle.

That's PaaS. It's a thin service wrapper (~320 lines) that bridges prover-core
into the Service Framework (SF), so provers can be launched, monitored, and
controlled alongside the rest of the node's services.

## How it fits together

```
prover-core                          paas
┌──────────────────────────┐        ┌──────────────────────────────┐
│ ProofSpec trait           │        │ ProverServiceBuilder          │
│ Prover<S>                │───────▶│ ProverHandle<S>               │
│ ProverBuilder             │        │ SF service (async, ticking)   │
│ ProveStrategy (nat/rem)   │        └──────────────────────────────┘
│ TaskStore, ReceiptStore   │
└──────────────────────────┘
 Knows: proving, retries,            Knows: SF lifecycle, command
 task lifecycle, recovery            routing, tick scheduling
```

prover-core does all the real work. PaaS just gives it a place to live.

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

The tick interval controls how often PaaS calls `prover.tick()` to scan for
retriable tasks and perform startup recovery. If you don't set one, the service
runs in command-only mode — no retries, no recovery, just direct commands. Good
for one-shot provers in tests.

### Using the handle

`ProverHandle<S>` is what consumers hold onto. It's generic over the spec only —
the zkVM host type is already erased inside the prover.

The entire API is keyed by your domain task type — no UUIDs, no intermediate
identifiers. You submit a task, wait for that task, get the receipt by that task.

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
    handle.submit(c.clone()).await?;  // idempotent — double-submit is a no-op
}
handle.wait_for_tasks(&chunks).await?;

// Get receipt by task — no UUID lookup needed
let receipt = handle.get_receipt(&chunks[0])?;
```

The full API:

| Method | Description |
|--------|-------------|
| `submit(task)` | Spawn a background prove. Idempotent — submitting the same task twice is a no-op. |
| `execute(task)` | Submit + block until done. Returns `TaskResult<Task>`. |
| `wait_for_tasks(tasks)` | Block until all tasks reach a terminal state. Watch-channel based, zero-poll. |
| `get_receipt(task)` | Read the stored receipt (requires a configured `ReceiptStore`). |
| `get_status(task)` | Current task status. |

`submit` and `execute` go through the SF command channel. `wait_for_tasks`,
`get_receipt`, and `get_status` read directly from shared prover state — no channel round-trip.

## Real-world examples

### OL checkpoint prover

Sequential, one epoch at a time. Uses a `ReceiptHook` to side-write proofs into
the domain's ProofDB.

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

Fan-out chunks in parallel, barrier, then aggregate. The shared receipt store is
the glue — the acct spec reads chunk receipts during `fetch_input`.

```rust
let receipt_store = Arc::new(SledReceiptStore::new(db));

// Chunk prover writes receipts
let chunk_prover = ProverBuilder::new(ChunkSpec { block_storage })
    .receipt_store(receipt_store.clone())
    .native(EeChunkProgram::native_host());
let chunk_handle = ProverServiceBuilder::new(chunk_prover)
    .launch(&executor).await?;

// Acct prover reads chunk receipts in fetch_input
let acct_prover = ProverBuilder::new(AcctSpec { batch_storage, receipt_store: receipt_store.clone() })
    .receipt_store(receipt_store)
    .native(EeAcctProgram::native_host());
let acct_handle = ProverServiceBuilder::new(acct_prover)
    .launch(&executor).await?;

// Orchestrate: fan-out chunks → barrier → aggregate
let chunks: Vec<_> = (0..num_chunks)
    .map(|i| ChunkTask { batch_id, chunk_idx: i })
    .collect();
join_all(chunks.iter().map(|c| chunk_handle.submit(c.clone()))).await;
chunk_handle.wait_for_tasks(&chunks).await?;
acct_handle.execute(AcctTask { batch_id }).await?;
```

### Switching to remote proving

The spec stays identical. Only the builder call changes:

```rust
let prover = ProverBuilder::new(spec)
    .receipt_store(sled_store)
    .task_store(SledTaskStore::open(&db)?)
    .retry(RetryConfig::default())
    .remote(sp1_host);   // instead of .native(host)
```

Requires the `remote` feature on prover-core.

## Feature status

### Implemented

- **Service Framework integration** — two service modes, both fully wired:
  - *Command-only* — no ticking, commands only. Good for one-shot provers and tests.
  - *Ticking* — commands + periodic `prover.tick()` for retry scanning and crash recovery.
- **Command routing** — `Submit` and `Execute` commands flow through a typed async
  channel. Completion senders return results directly to the caller.
- **ProverHandle** — cloneable, generic over spec only. Five methods: `submit`,
  `execute`, `wait_for_tasks`, `get_receipt`, `get_status`. Channel-based for
  commands, direct-read for queries (zero-copy, no round-trip).
- **Tick scheduling** — configurable interval via the builder. Drives retries and
  recovery without background threads.
- **Service status** — reports task count via `get_status()`. Available on both
  service modes.

### Planned

- **Health check API** — the `ServiceMonitor` is already held internally but not
  exposed on the handle. A dedicated `health()` method would let consumers and
  orchestrators check liveness without going through the command channel.
- **Graceful shutdown** — coordinated drain of in-flight tasks before service teardown,
  so remote proofs aren't abandoned mid-poll.
- **RPC bridge** — a thin adapter to expose `submit`/`execute`/`get_receipt` over
  JSON-RPC or gRPC, so provers can be driven from external tooling.

## What PaaS does NOT do

- **Proving** — that's prover-core's strategy layer.
- **Pipeline orchestration** — consumer code decides what depends on what.
- **Receipt storage logic** — prover-core's `ReceiptStore` and `ReceiptHook`.
- **RPC** — the binary crate's concern.
