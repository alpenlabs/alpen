# PaaS Design Documentation

**Last Updated:** 2025-11-04

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [State Machine Design](#state-machine-design)
3. [Component Architecture](#component-architecture)
4. [Service Pattern](#service-pattern)
5. [Worker Pool Design](#worker-pool-design)
6. [Error Handling](#error-handling)
7. [Design Decisions](#design-decisions)

---

## Architecture Overview

PaaS follows Strata's command worker pattern, providing an embeddable proof generation service with proper lifecycle management.

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Client Application                      │
│                  (strata-client / prover-client)            │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          │ ProverHandle
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                      ProverService                           │
│                   (AsyncService impl)                        │
│  ┌─────────────────────────────────────────────────────┐   │
│  │            ProverServiceState                        │   │
│  │  - TaskTracker (state machine)                      │   │
│  │  - ProofOperator (proof generation)                 │   │
│  │  - ProofDatabase (storage)                          │   │
│  │  - Configuration                                     │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                      Worker Pool                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │ Native Worker│  │ Native Worker│  │  SP1 Worker  │     │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
│           │                 │                 │             │
│           └─────────────────┴─────────────────┘             │
│                          │                                   │
│                          ▼                                   │
│              ┌────────────────────────┐                     │
│              │   ProofOperatorTrait   │                     │
│              │  - CheckpointOperator  │                     │
│              │  - ClStfOperator       │                     │
│              │  - EvmEeOperator       │                     │
│              └────────────────────────┘                     │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
                   ┌──────────────┐
                   │ ProofDatabase│
                   └──────────────┘
```

---

## State Machine Design

### Task Lifecycle States

```
┌──────────────────┐
│ WaitingForDeps   │
│  (has deps)      │
└────────┬─────────┘
         │ dependencies resolved
         ▼
┌──────────────────┐
│    Pending       │ ◄─── Initial state (no deps)
└────────┬─────────┘
         │ worker picks up task
         ▼
┌──────────────────┐
│     Queued       │
└────────┬─────────┘
         │ starts proving
         ▼
┌──────────────────┐
│    Proving       │
└────────┬─────────┘
         │
    ┌────┴────┐
    │         │
    ▼         ▼
┌────────┐  ┌─────────────────┐
│Complete│  │TransientFailure │
└────────┘  └────────┬────────┘
                     │ retry
                     ▼
            ┌──────────────────┐
            │     Queued        │
            └──────────────────┘
                     │
                     │ max retries exceeded
                     ▼
            ┌──────────────────┐
            │     Failed        │
            └──────────────────┘
```

### Valid State Transitions

Enforced by `TaskTracker::update_status()`:

| From State          | To State          | Trigger                    |
|---------------------|-------------------|----------------------------|
| WaitingForDeps      | Pending           | Dependencies resolved      |
| Pending             | Queued            | Worker pool picks up task  |
| Queued              | Proving           | Worker starts proof gen    |
| Proving             | Completed         | Proof generation succeeds  |
| Proving             | TransientFailure  | Recoverable error occurs   |
| TransientFailure    | Queued            | Retry triggered            |
| Any                 | Failed            | Permanent failure/max retries |

### Critical Bug Fixed (2025-11-04)

**Problem:** Worker pool was attempting invalid direct transitions:
- `Pending → Completed` ❌
- `Pending → TransientFailure` ❌

**Solution:** Implemented proper state progression:
- `Pending → Queued → Proving → Completed` ✅

**Impact:** Test pass rate improved from 12.5% to 81% (14/16 tests fixed)

**Files Modified:**
- `commands.rs`: Added `MarkQueued`, `MarkProving` commands
- `handle.rs`: Added `mark_queued()`, `mark_proving()` methods
- `service.rs`: Added command handlers
- `state.rs`: Added state transition methods
- `worker_pool.rs`: Fixed to call transitions in correct order

---

## Component Architecture

### Core Components

#### 1. ProverService

**Location:** `src/service.rs`

Implements Strata's `AsyncService` trait:
- `on_launch()` - Initialize service
- `process_input()` - Handle PaaSCommand messages
- `before_shutdown()` - Cleanup

**Command Types:**
- `CreateTask` - Submit new proof task
- `GetTaskStatus` - Query task state
- `GetProof` - Retrieve completed proof
- `GetReport` - Get service metrics
- `ListTasks` - List tasks by filter
- `MarkQueued` / `MarkProving` / `MarkCompleted` - State transitions
- `MarkTransientFailure` / `MarkFailed` - Error handling

#### 2. ProverServiceState

**Location:** `src/state.rs`

Manages service state:
- **TaskTracker** - State machine for task lifecycle
- **ProofOperator** - Delegated proof generation
- **ProofDatabase** - Persistent storage
- **Configuration** - Worker pools, retry policy
- **Statistics** - Cumulative metrics

**Key Methods:**
- `create_task()` - Register new proof task
- `mark_queued()` / `mark_proving()` / `mark_completed()` - State transitions
- `get_proof()` - Retrieve proof with borsh serialization
- `generate_report()` - Service metrics

#### 3. ProverHandle

**Location:** `src/handle.rs`

Public API for interacting with ProverService:

```rust
pub struct ProverHandle {
    command_handle: CommandHandle<PaaSCommand>,
    monitor: ServiceMonitor<PaaSStatus>,
}
```

**Methods:**
- `create_task()` - Async task submission
- `get_task_status()` - Status query
- `get_proof()` - Proof retrieval
- `mark_queued()` / `mark_proving()` / `mark_completed()` - State management
- `status_rx()` - Watch channel for status updates

#### 4. ProverBuilder

**Location:** `src/builder.rs`

Fluent API for service construction:

```rust
let handle = ProverBuilder::new()
    .with_config(config)
    .with_proof_operator(operator)
    .with_database(database)
    .launch(&executor)?;
```

**Responsibilities:**
- Validate dependencies
- Create service state
- Launch AsyncService
- Return ProverHandle

#### 5. TaskTracker

**Location:** `src/manager/task_tracker.rs`

Core state machine implementation:
- Maintains task metadata and state
- Enforces valid state transitions
- Manages dependency resolution
- Tracks retry counters

**Data Structures:**
```rust
pub struct TaskTracker {
    tasks: HashMap<TaskId, TaskMetadata>,
    pending: Vec<TaskId>,
    queued: Vec<TaskId>,
    proving: Vec<TaskId>,
    retriable: HashMap<TaskId, RetryInfo>,
}
```

#### 6. WorkerPool

**Location:** `src/manager/worker_pool.rs`

Manages proof generation workers:
- Polls for pending/retriable tasks
- Respects worker count limits per backend
- Spawns async proof generation tasks
- Handles state transitions and errors

**Worker Lifecycle:**
1. Poll `list_pending_tasks()` and `list_retriable_tasks()`
2. Check worker limit for backend
3. Spawn async task:
   - Call `mark_queued()`
   - Call `mark_proving()`
   - Call `operator.process_proof()`
   - Call `mark_completed()` or `mark_transient_failure()`

---

## Service Pattern

PaaS follows Strata's **Command Worker Pattern**:

### Pattern Components

1. **Service** - Defines state, message, and status types
2. **AsyncService** - Lifecycle hooks (launch, process_input, shutdown)
3. **ServiceBuilder** - Constructs and launches service
4. **CommandHandle** - Send commands and await responses
5. **ServiceMonitor** - Watch status updates

### Benefits

- **Type Safety** - Strong typing for commands and responses
- **Lifecycle Management** - Automatic via service framework
- **Status Broadcasting** - Built-in via watch channels
- **Graceful Shutdown** - Handled by framework
- **Testability** - Easy to mock CommandHandle

### Example Flow

```
Client                    Handle                   Service
  │                         │                         │
  │ create_task()           │                         │
  ├────────────────────────>│                         │
  │                         │ PaaSCommand::CreateTask │
  │                         ├────────────────────────>│
  │                         │                         │ process_input()
  │                         │                         ├─────────────┐
  │                         │                         │ creates task│
  │                         │                         │<────────────┘
  │                         │    Result<TaskId>       │
  │                         │<────────────────────────┤
  │   TaskId                │                         │
  │<────────────────────────┤                         │
  │                         │                         │
```

---

## Worker Pool Design

### Worker Management

**Configuration:**
```rust
pub struct WorkerConfig {
    pub worker_count: HashMap<ProofZkVm, usize>,
    pub polling_interval_ms: u64,
}
```

**Worker Limits:**
- Native backend: Configurable (e.g., 4 workers)
- SP1 backend: Configurable (e.g., 2 workers)

### Polling Strategy

**Loop:**
1. Fetch pending tasks
2. Fetch retriable tasks (with backoff)
3. For each task:
   - Check backend worker limit
   - If available, spawn proof generation task
   - Track in-progress count

**Interval:** Configurable (default: 100ms)

### Proof Generation Flow

```
WorkerPool
    │
    ├─> Spawn Task 1 ──> mark_queued()
    │                 └─> mark_proving()
    │                 └─> process_proof()
    │                 └─> mark_completed()
    │
    ├─> Spawn Task 2 ──> mark_queued()
    │                 └─> mark_proving()
    │                 └─> process_proof()
    │                 └─> mark_transient_failure()
    │
    └─> Spawn Task 3 ──> (same pattern)
```

---

## Error Handling

### Error Classification

**Transient Failures** (will retry):
- Network timeouts
- Temporary resource exhaustion
- Database lock contention
- Proof backend temporary unavailability

**Permanent Failures** (will not retry):
- Invalid proof context
- Unsupported VM type
- Malformed proof data
- Database corruption

**Current Implementation:**
- All errors treated as transient (conservative approach)
- TODO: Implement error classification logic

### Retry Policy

**Configuration:**
```rust
pub struct RetryConfig {
    pub max_retries: usize,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}
```

**Strategy:**
- Exponential backoff
- Max retry limit (default: 3)
- Min delay: 100ms
- Max delay: 5000ms

**Formula:**
```
delay = min(base_delay * 2^retry_count, max_delay)
```

### Error Propagation

1. **Worker Level:** Catch error, mark task as `TransientFailure`
2. **TaskTracker Level:** Update retry count, move to retriable queue
3. **Service Level:** Broadcast status update
4. **Client Level:** Poll status, observe failure/retry

---

## Design Decisions

### 1. Command Pattern over Direct Method Calls

**Decision:** Use command messages via ProverHandle instead of direct state access

**Rationale:**
- Type-safe async communication
- Decouples client from service internals
- Enables future RPC/network boundary
- Follows Strata service patterns

### 2. Separate WorkerPool from Service

**Decision:** WorkerPool polls ProverService for tasks

**Rationale:**
- Separation of concerns (scheduling vs state management)
- ProverService remains passive
- Easy to test independently
- Can add multiple worker pools

### 3. State Machine in TaskTracker

**Decision:** Centralize state transitions in TaskTracker

**Rationale:**
- Single source of truth for task state
- Enforces valid transitions
- Easy to audit state changes
- Prevents race conditions

### 4. Borsh Serialization for Proofs

**Decision:** Use borsh::to_vec() for ProofReceiptWithMetadata

**Rationale:**
- Efficient binary serialization
- Used throughout Strata codebase
- Deterministic (important for proofs)
- Fast serialization/deserialization

### 5. Conservative Error Handling

**Decision:** Treat all errors as transient initially

**Rationale:**
- Safer to retry than fail permanently
- Proof generation is idempotent
- Max retry limit prevents infinite loops
- Can refine classification later

### 6. Worker Limits per Backend

**Decision:** Separate worker count config for Native/SP1

**Rationale:**
- Different resource requirements
- SP1 proofs are resource-intensive
- Native proofs are lightweight
- Prevents resource starvation

### 7. Async Proof Generation

**Decision:** Spawn async tasks for proof generation

**Rationale:**
- Non-blocking worker pool
- Parallel proof generation
- Better resource utilization
- Fits Rust async ecosystem

---

## Future Enhancements

### Phase 3 (Planned)

1. **Error Classification**
   - Implement transient vs permanent distinction
   - Add error categorization logic
   - Reduce unnecessary retries

2. **Task Cancellation**
   - Worker coordination for in-flight cancellation
   - Graceful resource cleanup
   - Cancelled state support

3. **Duration Tracking**
   - Record task start/end times
   - Calculate average durations
   - Performance metrics per backend

4. **Public Values Extraction**
   - Extract public values from ProofReceipt
   - Expose in ProofData
   - Support verification use cases

5. **Checkpoint Runner**
   - Autonomous checkpoint proving
   - Integrate with PaaS service
   - Submit proofs to sequencer

---

## References

- **Strata Service Framework:** `crates/service/`
- **TaskTracker State Machine:** `crates/paas/src/manager/task_tracker.rs:54-82`
- **Worker Pool:** `crates/paas/src/manager/worker_pool.rs`
- **Command Pattern:** `crates/paas/src/commands.rs`
