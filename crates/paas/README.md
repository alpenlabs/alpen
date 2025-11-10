# Strata PaaS - Prover as a Service

General-purpose framework for managing proof generation with worker pools, retry logic, and lifecycle management.

## Architecture

### Core Abstraction

The `Prover` trait is the main abstraction point:

```rust
pub trait Prover: Send + Sync + 'static {
    type TaskId: TaskId;
    type Backend: Clone + Eq + Hash + Debug + Send + Sync + 'static;

    fn backend(&self, task_id: &Self::TaskId) -> Self::Backend;
    fn prove(&self, task_id: Self::TaskId) -> impl Future<Output = PaaSResult<()>> + Send;
}
```

The caller implements this trait to define:
- **Task ID type**: Any type that's hashable, serializable, and unique
- **Backend type**: Identifier for worker pooling (e.g., "sp1", "native")
- **backend()**: Maps tasks to backends for worker allocation
- **prove()**: The actual proving logic (completely opaque to PaaS)

### Task Lifecycle

```
Pending → Queued → Proving → Completed
                           ↓
                    TransientFailure → (retry) → Queued
                           ↓
                    PermanentFailure
```

- **Pending**: Task submitted, waiting for worker
- **Queued**: Assigned to backend queue
- **Proving**: Worker is generating proof
- **Completed**: Proof generated successfully
- **TransientFailure**: Retriable error (network, timeout, etc.)
- **PermanentFailure**: Non-retriable error (invalid input, etc.)

### Worker Pool Management

- Each backend has its own worker pool with configurable size
- Workers poll for pending/retriable tasks
- Worker limits prevent resource exhaustion
- RAII guards ensure proper cleanup

### Retry Logic

- Exponential backoff with configurable parameters
- Transient failures are retried up to max_retries
- Permanent failures are not retried
- Delay calculation: `base_delay * multiplier^retry_count` (capped at max_delay)

## Usage Example

```rust
use strata_paas::{Prover, ProverServiceBuilder, PaaSConfig, PaaSError, PaaSResult};

// 1. Define your task ID and backend types
#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
enum ProofBackend {
    Native,
    SP1,
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
struct ProofTask {
    id: u64,
    proof_type: String,
}

// 2. Implement the Prover trait
struct MyProver {
    // Your proving infrastructure (clients, databases, etc.)
}

impl Prover for MyProver {
    type TaskId = ProofTask;
    type Backend = ProofBackend;

    fn backend(&self, task: &ProofTask) -> ProofBackend {
        match task.proof_type.as_str() {
            "fast" => ProofBackend::Native,
            "secure" => ProofBackend::SP1,
            _ => ProofBackend::Native,
        }
    }

    async fn prove(&self, task: ProofTask) -> PaaSResult<()> {
        // Your proving logic here
        // Return transient/permanent errors as appropriate
        Ok(())
    }
}

// 3. Launch the service
let prover = Arc::new(MyProver { /* ... */ });
let config = PaaSConfig::new(HashMap::from([
    (ProofBackend::Native, 5),
    (ProofBackend::SP1, 20),
]));

let handle = ProverServiceBuilder::new()
    .with_prover(prover)
    .with_config(config)
    .launch(&executor)?;

// 4. Use the handle to submit tasks and check status
handle.submit_task(ProofTask { id: 1, proof_type: "fast".into() }).await?;
let status = handle.get_status(&task_id).await?;
```

## Design Principles

1. **Separation of Concerns**: PaaS handles lifecycle/pooling, caller handles proving
2. **Type Safety**: Generic over task ID and backend types
3. **Flexibility**: Works with any proof system (SP1, RISC0, native, etc.)
4. **Observability**: Built-in status reporting and metrics
5. **Reliability**: Retry logic, worker limits, graceful shutdown

## Files

- `lib.rs` - Public API and Prover trait
- `task.rs` - Task types and lifecycle
- `error.rs` - Error types
- `config.rs` - Configuration types
- `state.rs` - Service state management
- `service.rs` - Service implementation (AsyncService pattern)
- `worker.rs` - Worker pool implementation
- `handle.rs` - Command handle for API
- `builder.rs` - Builder pattern for service creation
