# Strata PaaS - Prover as a Service

General-purpose framework for managing proof generation with worker pools, retry logic, and lifecycle management.

## Architecture

### Core Abstraction

The `Prover` trait is the main abstraction point:

```rust
pub trait Prover: Send + Sync + 'static {
    type TaskId: TaskIdentifier;
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

## Usage Example - Registry Pattern (Recommended)

The registry pattern provides better type safety and extensibility by allowing you to register
multiple program handlers dynamically:

```rust
use strata_paas::registry::RegistryProverServiceBuilder;
use strata_paas::PaaSConfig;

// Define your program type with routing
enum MyProgram {
    TypeA(u64),
    TypeB(String),
}

enum MyProgramVariant {
    TypeA,
    TypeB,
}

impl ProgramType for MyProgram {
    type RoutingKey = MyProgramVariant;

    fn routing_key(&self) -> Self::RoutingKey {
        match self {
            MyProgram::TypeA(_) => MyProgramVariant::TypeA,
            MyProgram::TypeB(_) => MyProgramVariant::TypeB,
        }
    }
}

// Launch with registry
let handle = RegistryProverServiceBuilder::new(config)
    .register::<ProgramA, _, _, _>(
        MyProgramVariant::TypeA,
        input_fetcher_a,
        proof_store,
        host_a,
    )
    .register::<ProgramB, _, _, _>(
        MyProgramVariant::TypeB,
        input_fetcher_b,
        proof_store,
        host_b,
    )
    .launch(&executor)
    .await?;

// Submit tasks with clean API
handle.submit_task(MyProgram::TypeA(42), ZkVmBackend::SP1).await?;
```

See the `registry` module documentation for complete examples.

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
