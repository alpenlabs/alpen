# Prover Client - PaaS Integration Guide

This document describes the integration of the Strata PaaS (Prover-as-a-Service) framework into the prover-client binary.

## Overview

The prover-client has been refactored to use the PaaS framework for managing proof generation. This provides:

- **Worker pool management** for Native and SP1 backends
- **Automatic retry logic** with exponential backoff
- **Task lifecycle tracking** (Pending → Queued → Proving → Completed/Failed)
- **Graceful shutdown** with proper cleanup
- **Status monitoring** via watch channels

## Architecture

### High-Level Flow

```
RPC Request → ProverHandle → PaaS Service → Worker Pool → zkaleido → Proof Store
                    ↓                              ↓
              Status Query                    Proof Generation
```

### Key Components

#### 1. `ZkVmProver` (paas_integration.rs)

The main implementation of the `Prover` trait that bridges PaaS with zkaleido:

```rust
pub struct ZkVmProver<P: ProgramId> {
    proof_store: Arc<ProofStore<P>>,  // Database for storing proofs
    phantom: PhantomData<P>,
}
```

**Responsibilities:**
- Resolves zkVM backend (Native vs SP1) from task
- Loads zkVM host instances (lazily initialized, cached)
- Generates proofs using zkaleido's `ProverProgram::prove()`
- Stores completed proofs in the database
- Handles error classification (transient vs permanent)

#### 2. `ProofStore` (paas_integration.rs)

Database adapter that maps PaaS task IDs to proof storage:

```rust
pub struct ProofStore<P: ProgramId> {
    db: Arc<ProofDBSled>,
    phantom: PhantomData<P>,
}
```

**Methods:**
- `get_proof()` - Retrieve proof from database
- `store_proof()` - Store completed proof
- `get_proof_deps()` - Get proof dependencies for a program

#### 3. `ProverHandle` (crates/paas/src/handle.rs)

Public API for interacting with the prover service:

```rust
pub struct ProverHandle<P: ProgramId> {
    command_handle: Arc<CommandHandle<ProverCommand<ZkVmTaskId<P>>>>,
    monitor: ServiceMonitor<ProverServiceStatus>,
}
```

**API Methods:**
- `submit_task(task_id) → Result<()>` - Submit new proof task
- `get_status(task_id) → Result<TaskStatus>` - Query task status
- `cancel_task(task_id) → Result<()>` - Cancel pending/queued task
- `status_rx() → watch::Receiver<Status>` - Subscribe to status updates

#### 4. Task Types

**ZkVmTaskId:**
```rust
pub struct ZkVmTaskId<P: ProgramId> {
    pub program: P,      // Program identifier (e.g., ProofContext)
    pub backend: ZkVmBackend,  // Native or SP1
}
```

**ZkVmBackend:**
```rust
pub enum ZkVmBackend {
    Native,
    SP1,
    Risc0,  // Not currently supported
}
```

### Integration Points

#### Main Binary (main.rs)

```rust
// Create proof store
let proof_store = Arc::new(ProofStore::new(db.clone()));

// Create zkVM prover
let zkvm_prover = Arc::new(ZkVmProver::new(proof_store));

// Configure worker pools
let mut worker_limits = HashMap::new();
worker_limits.insert(ZkVmBackend::Native, 5);
#[cfg(feature = "sp1")]
worker_limits.insert(ZkVmBackend::SP1, 20);

let paas_config = PaaSConfig::new(worker_limits)
    .with_polling_interval(Duration::from_secs(config.polling_interval))
    .with_retry_config(RetryConfig::new(
        config.max_retry_counter,
        Duration::from_secs(10),
        2.0,
        Duration::from_secs(300),
    ));

// Launch PaaS service
let prover_handle = ProverServiceBuilder::new()
    .with_prover(zkvm_prover)
    .with_config(paas_config)
    .launch(&executor)?;

// Use handle in RPC server and checkpoint runner
```

#### RPC Server (rpc_server.rs)

The RPC server uses `ProverHandle` to submit tasks:

```rust
async fn prove_checkpoint(&self, ckp_idx: u64) -> RpcResult<Vec<ProofKey>> {
    let proof_ctx = ProofContext::Checkpoint(ckp_idx);

    // Determine backend
    let backend = if cfg!(feature = "sp1") {
        ZkVmBackend::SP1
    } else {
        ZkVmBackend::Native
    };

    // Get dependencies
    let deps = self.db.get_proof_deps(proof_ctx)?;

    // Submit dependency tasks first
    for dep_ctx in &deps {
        let dep_task_id = ZkVmTaskId {
            program: *dep_ctx,
            backend: backend.clone(),
        };
        self.prover_handle.submit_task(dep_task_id).await?;
    }

    // Submit main task
    let task_id = ZkVmTaskId {
        program: proof_ctx,
        backend,
    };
    self.prover_handle.submit_task(task_id).await?;

    Ok(vec![ProofKey::new(proof_ctx, zkvm)])
}
```

#### Checkpoint Runner (checkpoint_runner/runner.rs)

Autonomous checkpoint proving using `ProverHandle`:

```rust
pub(crate) async fn checkpoint_proof_runner(
    operator: CheckpointOperator,
    poll_interval_s: u64,
    prover_handle: ProverHandle<ProofContext>,
    db: Arc<ProofDBSled>,
) {
    let mut ticker = interval(Duration::from_secs(poll_interval_s));

    loop {
        ticker.tick().await;

        // Fetch next unproven checkpoint
        let checkpoint_idx = fetch_next_unproven_checkpoint_index(&operator).await?;

        // Submit proof task with dependencies
        submit_checkpoint_task(checkpoint_idx, &prover_handle, &db).await?;
    }
}

async fn submit_checkpoint_task(
    checkpoint_idx: u64,
    prover_handle: &ProverHandle<ProofContext>,
    db: &Arc<ProofDBSled>,
) -> anyhow::Result<()> {
    let proof_ctx = ProofContext::Checkpoint(checkpoint_idx);
    let backend = get_backend();  // SP1 if feature enabled, else Native

    // Check if proof already exists
    let proof_key = ProofKey::new(proof_ctx, zkvm_from_backend(backend));
    if db.get_proof(&proof_key)?.is_some() {
        return Ok(());  // Already proven
    }

    // Get and submit dependencies
    let deps = db.get_proof_deps(proof_ctx)?.unwrap_or_default();
    for dep_ctx in &deps {
        let dep_task_id = ZkVmTaskId {
            program: *dep_ctx,
            backend: backend.clone(),
        };
        prover_handle.submit_task(dep_task_id).await?;
    }

    // Submit main task
    let task_id = ZkVmTaskId {
        program: proof_ctx,
        backend,
    };
    prover_handle.submit_task(task_id).await?;

    Ok(())
}
```

## Task Lifecycle

### 1. Task Submission

```
submit_task(task_id)
    ↓
Check if already completed (in DB)
    ↓ No
Check if already pending/proving (in PaaS state)
    ↓ No
Create task with status = Pending
    ↓
Add to task queue
    ↓
Return success
```

### 2. Worker Processing

```
Worker polls for tasks
    ↓
Find Pending or retriable task
    ↓
Update status → Queued
    ↓
Acquire worker slot
    ↓
Update status → Proving
    ↓
Call prover.prove(task_id)
    ↓
    ├─ Success → Update status → Completed
    │              Store proof in DB
    │
    ├─ Transient Error → Update status → TransientFailure
    │                     Increment retry count
    │                     Calculate backoff delay
    │
    └─ Permanent Error → Update status → PermanentFailure
                          No retry
```

### 3. Retry Logic

**Transient failures are retried with exponential backoff:**

```rust
delay = base_delay * multiplier^retry_count
delay = min(delay, max_delay)
```

**Default configuration:**
- `base_delay`: 10 seconds
- `multiplier`: 2.0
- `max_delay`: 5 minutes
- `max_retries`: 3

**Example:**
- Retry 0: 10s delay
- Retry 1: 20s delay
- Retry 2: 40s delay
- Retry 3: 80s delay (capped at max_delay)

### 4. Status Queries

```
get_status(task_id)
    ↓
Check DB for completed proof
    ↓ Found
Return Completed
    ↓ Not found
Check PaaS task state
    ↓ Found
Return current status (Pending/Queued/Proving/Failed)
    ↓ Not found
Return error: Task not found
```

## Error Handling

### Error Classification

**Transient Errors** (will retry):
- Network timeouts
- Resource temporarily unavailable
- Database deadlocks
- SP1 proving timeouts
- Worker panics

**Permanent Errors** (no retry):
- Invalid proof context
- Missing dependencies (after all retries)
- Invalid input data
- Proof verification failed
- Backend not supported

### Error Propagation

```rust
// In ZkVmProver::prove()
match ProverProgram::prove(&input, &host) {
    Ok(proof) => {
        // Store proof
        self.proof_store.store_proof(&task_id, proof).await?;
        Ok(())
    }
    Err(e) if is_transient(&e) => {
        Err(PaaSError::TransientFailure(format!("Proving failed: {}", e)))
    }
    Err(e) => {
        Err(PaaSError::PermanentFailure(format!("Proving failed: {}", e)))
    }
}
```

## Configuration

### Worker Pool Limits

```rust
let mut worker_limits = HashMap::new();
worker_limits.insert(ZkVmBackend::Native, 5);   // 5 Native workers
worker_limits.insert(ZkVmBackend::SP1, 20);     // 20 SP1 workers
```

**Rationale:**
- Native proving is fast, so fewer workers needed
- SP1 proving is slower, so more workers for parallelism
- Prevents resource exhaustion (memory, CPU)

### Polling Interval

```rust
.with_polling_interval(Duration::from_secs(5))
```

**Default:** 5 seconds

Workers check for new tasks every polling interval.

### Retry Configuration

```rust
.with_retry_config(RetryConfig::new(
    3,                            // max_retries
    Duration::from_secs(10),      // base_delay
    2.0,                          // multiplier
    Duration::from_secs(300),     // max_delay
))
```

## Database Schema

### Proof Storage

Proofs are stored in the database keyed by `ProofKey`:

```rust
pub struct ProofKey {
    pub context: ProofContext,   // e.g., Checkpoint(123)
    pub zkvm: ProofZkVm,          // Native or SP1
}
```

**Tables:**
- `proofs`: Completed proofs (key → proof data)
- `proof_deps`: Proof dependencies (context → [dep_contexts])

### Task State

Task state is **not persisted** in the database - it's maintained in-memory by PaaS. On restart:
1. All in-flight tasks are lost
2. Completed proofs remain in database
3. Checkpoint runner will re-submit any unproven checkpoints

## Observability

### Status Monitoring

```rust
let status_rx = prover_handle.status_rx();

tokio::spawn(async move {
    while status_rx.changed().await.is_ok() {
        let status = status_rx.borrow();
        println!("Worker pools: {:?}", status.worker_pools);
        println!("Task counts: {:?}", status.task_counts);
    }
});
```

**Status fields:**
- `worker_pools`: Per-backend worker utilization
- `task_counts`: Tasks by status (Pending, Queued, Proving, etc.)
- `retry_counts`: Tasks by retry count
- `last_update`: Timestamp of last status change

### Logging

PaaS uses `tracing` for structured logging:

```
INFO prover_service: Task submitted task_id=Checkpoint(123) backend=SP1
INFO prover_service: Worker started backend=SP1 worker_id=5
INFO prover_service: Task proving task_id=Checkpoint(123)
INFO prover_service: Task completed task_id=Checkpoint(123) duration=45s
WARN prover_service: Task failed (transient) task_id=Checkpoint(123) error="timeout" retry=1/3
ERROR prover_service: Task failed (permanent) task_id=Checkpoint(123) error="invalid input"
```

## Testing

### Unit Tests

See `crates/paas/tests/`:
- `config_tests.rs` - Retry logic configuration
- `task_tests.rs` - Task status predicates

### Integration Tests

Manual testing:
1. Start prover-client: `cargo run --bin strata-prover-client`
2. Submit checkpoint via RPC: `prove_checkpoint(123)`
3. Monitor logs for task progression
4. Query status: `get_task_status(ProofKey::new(Checkpoint(123), SP1))`
5. Verify proof in database

### Functional Tests

See `functional-tests/tests/prover/`:
- `prover_client_happy.py` - Basic proof generation
- `prover_client_restart.py` - Persistence across restarts

## Migration from TaskTracker

### Before (TaskTracker)

```rust
// Old approach: Direct task management
let task_tracker = Arc::new(Mutex::new(TaskTracker::new()));

// Submit task
let key = task_tracker.lock().await.create_tasks(context, deps, &db)?;

// ProverManager polls and spawns make_proof() tasks
let manager = ProverManager::new(task_tracker.clone(), operator, db, config);
spawn(async move { manager.process_pending_tasks().await });

// RPC queries TaskTracker directly
let status = task_tracker.lock().await.get_task(key)?;
```

**Problems:**
- Manual worker pool management
- No built-in retry logic
- Status tracking mixed with business logic
- Difficult to test in isolation

### After (PaaS)

```rust
// New approach: PaaS service
let prover_handle = ProverServiceBuilder::new()
    .with_prover(zkvm_prover)
    .with_config(paas_config)
    .launch(&executor)?;

// Submit task
prover_handle.submit_task(task_id).await?;

// Workers managed by PaaS
// Retry logic handled by PaaS
// Status tracking handled by PaaS

// RPC queries via handle
let status = prover_handle.get_status(&task_id).await?;
```

**Benefits:**
- Separation of concerns (PaaS handles lifecycle, zkvm_prover handles proving)
- Built-in retry with exponential backoff
- Worker pool management with limits
- Cleaner API (async/await, no mutexes in caller code)
- Easier testing (mock Prover trait)

## Performance Considerations

### Worker Pool Sizing

**Native workers:**
- Fast execution (< 1 second per proof typically)
- Low memory footprint
- CPU-bound
- **Recommendation:** 5-10 workers

**SP1 workers:**
- Slow execution (30s - 5min per proof)
- High memory footprint (8-16GB per worker)
- CPU + memory bound
- **Recommendation:** 10-20 workers depending on available memory

### Polling Interval

- **Too low** (< 1s): Wastes CPU checking for tasks
- **Too high** (> 30s): Delays task pickup, poor latency
- **Recommended:** 5 seconds for good balance

### Database Contention

- Proof storage uses optimistic concurrency (retry on conflict)
- Multiple workers can write proofs concurrently
- Dependency queries are read-only (no contention)

## Troubleshooting

### Task stuck in Pending

**Symptoms:** Task submitted but never transitions to Queued

**Possible causes:**
1. No workers for that backend (check worker pool config)
2. All workers busy with long-running tasks (check worker limits)
3. PaaS service not running (check process status)

**Solutions:**
- Increase worker pool size
- Check logs for worker panics
- Verify backend is enabled (e.g., SP1 feature)

### Task repeatedly failing (transient)

**Symptoms:** Task goes TransientFailure → Queued → Proving → TransientFailure

**Possible causes:**
1. Network issues (sequencer/reth RPC timeout)
2. Resource exhaustion (OOM, CPU throttling)
3. SP1 prover timeouts

**Solutions:**
- Check network connectivity
- Increase retry delay (give more time to recover)
- Reduce worker pool size (lower memory pressure)
- Check logs for specific error messages

### Task failed (permanent)

**Symptoms:** Task stuck in PermanentFailure status

**Possible causes:**
1. Invalid checkpoint index (doesn't exist)
2. Missing dependencies (proof deps not available)
3. Corrupt input data
4. Backend not supported (e.g., Risc0)

**Solutions:**
- Verify checkpoint exists in sequencer
- Check proof dependencies are completed
- Re-submit with correct parameters
- Use supported backend (Native or SP1)

### Proofs not being stored

**Symptoms:** Proofs complete but `get_proof()` returns None

**Possible causes:**
1. Database write error (check logs)
2. Proof store adapter bug
3. Wrong ProofKey used for lookup

**Solutions:**
- Check database permissions
- Verify ProofKey matches (context + zkvm)
- Check logs for storage errors

## Migration Cleanup

### Obsolete Code Removed

As part of the PaaS migration, the following legacy components were removed from prover-client:

#### Deleted Files (4 files, ~1,200 lines)

1. **`prover_manager.rs`** - Old worker pool manager
   - Manually spawned tokio tasks for each proof
   - No worker limits (risk of resource exhaustion)
   - Replaced by: PaaS worker pools with configurable limits

2. **`retry_policy.rs`** - Old retry logic
   - Ad-hoc retry implementation with constant backoff
   - Retry state tracked alongside task state
   - Replaced by: PaaS `RetryConfig` with exponential backoff

3. **`status.rs`** - Old task status types
   - `ProvingTaskStatus` enum (Pending, ProvingInProgress, Completed, etc.)
   - Manually tracked state transitions
   - Replaced by: PaaS `TaskStatus` with automatic lifecycle management

4. **`task_tracker.rs`** - Old task state machine
   - Wrapped in `Arc<Mutex<...>>` requiring locks for all operations
   - Manual dependency resolution
   - No graceful shutdown mechanism
   - Replaced by: PaaS `TaskManager` with lock-free API

#### Removed Abstractions (~650 lines)

1. **`ProvingOp` trait** - Task creation abstraction
   - `create_task()` - Created proof tasks with dependencies
   - `create_deps_tasks()` - Resolved proof dependencies
   - `construct_proof_ctx()` - Mapped params to ProofContext
   - `fetch_input()` - Fetched prover inputs
   - `prove()` - Executed proof generation
   - **Replaced by**: PaaS handles task creation, operators only provide `fetch_input()`

2. **Test code** - ProvingOp unit tests
   - GrandparentOps, ParentOps, ChildOps test stubs
   - Dependency chain tests
   - Task restart simulation tests
   - **No longer needed**: PaaS library has comprehensive tests

#### Simplified Operators

Operators now only provide data fetching utilities, not task management:

**CheckpointOperator:**
- ✅ `fetch_input(task_id, db)` - Fetch checkpoint proof input
- ✅ `fetch_ckp_info(idx)` - Fetch checkpoint info from CL client
- ✅ `cl_client()` - Get CL client reference
- ✅ `submit_checkpoint_proof()` - Submit proof to sequencer
- ❌ ~~`create_task()`~~ - Removed (PaaS handles this)
- ❌ ~~`create_deps_tasks()`~~ - Removed (PaaS handles this)
- ❌ ~~`construct_proof_ctx()`~~ - Removed (not needed)

**ClStfOperator:**
- ✅ `fetch_input(task_id, db)` - Fetch CL STF proof input
- ✅ `get_chainstate_before(blkid)` - Get chainstate before block
- ✅ `get_block(blkid)` - Get L2 block
- ❌ ~~`create_task()`~~ - Removed (PaaS handles this)
- ❌ ~~`create_deps_tasks()`~~ - Removed (PaaS handles this)
- ❌ ~~`construct_proof_ctx()`~~ - Removed (not needed)

**EvmEeOperator:**
- ✅ `fetch_input(task_id, db)` - Fetch EVM EE proof input
- ✅ `get_block_header_by_height(num)` - Get EVM block header
- ❌ ~~`create_task()`~~ - Removed (PaaS handles this)
- ❌ ~~`construct_proof_ctx()`~~ - Removed (not needed)

**ProofOperator:**
- ✅ `init()` - Initialize all operators
- ✅ `evm_ee_operator()` - Get EVM EE operator reference
- ✅ `cl_stf_operator()` - Get CL STF operator reference
- ✅ `checkpoint_operator()` - Get checkpoint operator reference
- ❌ ~~`prove()`~~ - Removed (PaaS handles proving)
- ❌ ~~`process_proof()`~~ - Removed (PaaS handles dispatching)

### Migration Benefits

**Before (TaskTracker + ProverManager):**
```
RPC → create_task() → TaskTracker.lock()
                    → create_deps_tasks()
                    → ProverManager spawns tokio task
                    → Manual retry with constant backoff
                    → No worker limits
```

**After (PaaS):**
```
RPC → ProverHandle.submit_task()
    → PaaS queues task
    → Worker pool picks up task
    → Automatic retry with exponential backoff
    → Configurable worker limits
```

**Impact:**
- **-1,693 lines** of code (net reduction)
- **Simpler codebase**: Operators only fetch data, PaaS handles orchestration
- **Better concurrency**: Lock-free API, no mutex contention
- **More reliable**: Built-in retry logic, graceful shutdown
- **More configurable**: Worker limits, retry policies, status monitoring

## Future Improvements

### Planned Features

1. **Proof cancellation** - Cancel in-flight proving tasks
2. **Task prioritization** - Priority queue for urgent proofs
3. **Batch proving** - Prove multiple contexts in one task
4. **Progress reporting** - Stream proving progress (% complete)
5. **Metrics exporter** - Prometheus metrics for monitoring

### Performance Optimizations

1. **Proof caching** - Cache frequently used proofs in memory
2. **Lazy dependency resolution** - Only load deps when needed
3. **Worker affinity** - Pin workers to NUMA nodes for better performance
4. **Adaptive polling** - Adjust interval based on task arrival rate

## References

- PaaS library: `crates/paas/`
- PaaS integration: `bin/prover-client/src/paas_integration.rs`
- RPC implementation: `bin/prover-client/src/rpc_server.rs`
- Checkpoint runner: `bin/prover-client/src/checkpoint_runner/runner.rs`
- zkaleido: https://github.com/alpenlabs/zkaleido
