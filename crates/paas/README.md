# Prover-as-a-Service (PaaS)

A framework for managing zkVM proof generation tasks with worker pools, retry logic, and lifecycle management.

## Overview

PaaS provides a service for orchestrating zero-knowledge proof generation across multiple zkVM backends (SP1, Risc0, Native). Features:

- **Task lifecycle management**: Submit, track, and retrieve proof generation tasks
- **Worker pool concurrency**: Configurable worker pools per zkVM backend
- **Automatic retries**: Exponential backoff retry logic for transient failures
- **Persistent storage**: Task state and proof persistence
- **Host resolution**: Flexible zkVM host/program resolution
- **Command-based architecture**: Clean separation between service logic and external API

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      ProverHandle (API)                      │
│              submit_task(), execute_task(), get_status()     │
└──────────────────────┬──────────────────────────────────────┘
                       │ Commands
┌──────────────────────▼──────────────────────────────────────┐
│                    ProverService                             │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ ProverServiceState                                     │ │
│  │ • Task routing & execution                             │ │
│  │ • Handler dispatch                                     │ │
│  │ • Concurrency control (semaphores)                     │ │
│  │ • Retry coordination                                   │ │
│  └────────┬───────────────────┬──────────────┬────────────┘ │
└───────────┼───────────────────┼──────────────┼──────────────┘
            │                   │              │
    ┌───────▼───────┐   ┌──────▼──────┐  ┌───▼──────────────┐
    │ TaskStore     │   │ ProofHandler│  │ RetryScheduler   │
    │ (Persistent)  │   │ (Execution) │  │ (Retries)        │
    └───────────────┘   └─────────────┘  └──────────────────┘
```

### Key Components

- **ProverService**: Core service runtime managing proof generation lifecycle
- **ProverHandle**: External API for submitting and querying tasks
- **ProofHandler**: Trait for proof generation (fetch input → prove → store)
- **TaskStore**: Persistent storage for task tracking
- **RetryScheduler**: Background service for delayed retry scheduling

## Module Structure

```
src/
├── lib.rs           # Public API and re-exports
├── task.rs          # Task types (TaskId, TaskStatus, TaskResult)
├── service/         # Core service runtime
│   ├── runtime.rs   # ProverService implementation
│   ├── state.rs     # ProverServiceState (task routing & execution)
│   ├── commands.rs  # Command types for service communication
│   ├── handle.rs    # ProverHandle (external API)
│   └── builder.rs   # ProverServiceBuilder
├── scheduler/       # Retry scheduler for delayed task execution
│   └── scheduler.rs # RetryScheduler, SchedulerHandle, SchedulerCommand
├── handler/         # Proof generation
│   ├── traits.rs    # ProofHandler, InputFetcher, ProofStorer
│   ├── remote.rs    # RemoteProofHandler (zkaleido integration)
│   └── host.rs      # HostResolver, HostInstance
├── config.rs        # ProverServiceConfig, RetryConfig, WorkerConfig
├── program.rs       # ProgramType trait
├── error.rs         # Error types
└── persistence.rs   # TaskStore trait
```

**Design rationale:**
- **Fundamental types at root**: task, config, program, error, persistence are used everywhere
- **Domain modules**: service, scheduler, handler have focused responsibilities
- **Flat structure**: Easy to find and import commonly-used types

## Quick Start

### 1. Define Your Program Type

```rust
use strata_paas::ProgramType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MyProgram {
    Checkpoint(CheckpointInput),
    StateTransition(StateInput),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProgramVariant {
    Checkpoint,
    StateTransition,
}

impl ProgramType for MyProgram {
    type RoutingKey = ProgramVariant;

    fn routing_key(&self) -> Self::RoutingKey {
        match self {
            MyProgram::Checkpoint(_) => ProgramVariant::Checkpoint,
            MyProgram::StateTransition(_) => ProgramVariant::StateTransition,
        }
    }
}
```

### 2. Implement Required Traits

```rust
use strata_paas::{InputFetcher, ProofStorer, HostResolver};

// Fetch proof inputs
struct MyInputFetcher;
impl InputFetcher<MyProgram> for MyInputFetcher {
    async fn fetch_input(&self, program: &MyProgram) -> anyhow::Result<Vec<u8>> {
        // Fetch input data from your data source
        todo!()
    }
}

// Store completed proofs
struct MyProofStorer;
impl ProofStorer<MyProgram> for MyProofStorer {
    async fn store_proof(&self, program: &MyProgram, proof: Vec<u8>) -> anyhow::Result<()> {
        // Store proof to your storage backend
        todo!()
    }
}

// Resolve zkVM hosts
struct MyHostResolver;
impl HostResolver for MyHostResolver {
    fn resolve(&self, variant: &dyn Any, backend: &ZkVmBackend) -> anyhow::Result<HostInstance> {
        // Return appropriate zkVM host
        todo!()
    }
}
```

### 3. Build and Launch the Service

```rust
use strata_paas::{ProverServiceBuilder, ProverServiceConfig, RemoteProofHandler};

// Configure the service
let config = ProverServiceConfig::default()
    .with_sp1_workers(4)
    .with_risc0_workers(2)
    .with_retries(5, 10, 2.0, 300);

// Create handlers for each program variant
let checkpoint_handler = Arc::new(RemoteProofHandler::new(
    input_fetcher.clone(),
    proof_storer.clone(),
    host_resolver.clone(),
));

let state_handler = Arc::new(RemoteProofHandler::new(
    input_fetcher,
    proof_storer,
    host_resolver,
));

// Build and launch
let handle = ProverServiceBuilder::new(config)
    .with_task_store(task_store)
    .with_handler(ProgramVariant::Checkpoint, checkpoint_handler)
    .with_handler(ProgramVariant::StateTransition, state_handler)
    .launch(&executor)
    .await?;
```

### 4. Submit and Track Tasks

```rust
// Submit a task (fire-and-forget, returns UUID)
let uuid = handle.submit_task(
    MyProgram::Checkpoint(input),
    ZkVmBackend::SP1
).await?;

// Check status
let status = handle.get_status(&uuid).await?;

// Or execute and wait for completion
let result = handle.execute_task(
    MyProgram::StateTransition(input),
    ZkVmBackend::Risc0
).await?;
```

## Configuration

### Worker Pools

Configure concurrent workers per backend:

```rust
let config = ProverServiceConfig::default()
    .with_sp1_workers(4)      // 4 concurrent SP1 proofs
    .with_risc0_workers(2)    // 2 concurrent Risc0 proofs
    .with_native_workers(8);  // 8 concurrent native executions
```

### Retry Logic

Enable automatic retries with exponential backoff:

```rust
use strata_paas::RetryConfig;

let retry_config = RetryConfig {
    max_retries: 5,           // Maximum retry attempts
    base_delay_secs: 10,      // Initial delay (seconds)
    multiplier: 2.0,          // Exponential multiplier
    max_delay_secs: 300,      // Cap delay at 5 minutes
};

let config = ProverServiceConfig::default()
    .with_retry_config(retry_config);
```

Or use the convenience method:

```rust
let config = ProverServiceConfig::default()
    .with_retries(5, 10, 2.0, 300);
```

### Task Persistence

Implement `TaskStore` for task tracking:
```

## Advanced Usage

### Custom ProofHandler

For custom execution logic beyond `RemoteProofHandler`:

```rust
use strata_paas::ProofHandler;

struct CustomHandler {
    // Your custom state
}

#[async_trait]
impl ProofHandler<MyProgram> for CustomHandler {
    async fn handle_proof(&self, task_id: TaskId<MyProgram>) -> anyhow::Result<()> {
        // Custom proof generation logic:
        // 1. Fetch input
        // 2. Generate proof
        // 3. Store proof
        todo!()
    }
}
```

### Task Lifecycle

Tasks progress through these states:

1. **Pending**: Task submitted, awaiting execution
2. **InProgress**: Currently being proven
3. **Completed**: Proof generated and stored successfully
4. **Failed**: Proof generation failed (with retry count)

```rust
use strata_paas::TaskStatus;

match status {
    TaskStatus::Pending => println!("Waiting..."),
    TaskStatus::InProgress => println!("Proving..."),
    TaskStatus::Completed { .. } => println!("Done!"),
    TaskStatus::Failed { retry_count, .. } => println!("Failed (retry {})", retry_count),
}
```

### Monitoring

Get service status summary:

```rust
let summary = handle.get_current_status();
println!("Pending: {}", summary.pending);
println!("In Progress: {}", summary.in_progress);
println!("Completed: {}", summary.completed);
println!("Failed: {}", summary.failed);
```

## Design Patterns

### Command-Based Architecture

PaaS uses a command-based pattern for clean separation:

- **External API** (`ProverHandle`): Send commands to service
- **Service** (`ProverService`): Process commands asynchronously
- **No shared state**: All communication via commands/channels

This enables:
- Thread-safe concurrent access
- Clean service boundaries
- Easy testing and mocking
- Service restartability

### Handler Composition

Handlers are composable and injectable:

```rust
// Simple composition
let handler = RemoteProofHandler::new(fetcher, storer, resolver);

// Wrapped handlers for logging, metrics, etc.
struct LoggingHandler<H> {
    inner: H,
}

impl<P: ProgramType, H: ProofHandler<P>> ProofHandler<P> for LoggingHandler<H> {
    async fn handle_proof(&self, task_id: TaskId<P>) -> anyhow::Result<()> {
        info!("Starting proof: {:?}", task_id);
        let result = self.inner.handle_proof(task_id).await;
        info!("Finished proof: {:?}", result);
        result
    }
}
```

### Semaphore-Based Concurrency

Worker pools use semaphores for backpressure:

- Each backend has dedicated semaphore (capacity = worker count)
- Tasks acquire permit before execution
- Natural rate limiting and resource management
- Prevents overwhelming zkVM backends

## Testing

### Mock Components

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct MockTaskStore;

    #[async_trait]
    impl TaskStore<MyProgram> for MockTaskStore {
        // Implement with in-memory HashMap for testing
    }

    #[tokio::test]
    async fn test_service() {
        let store = Arc::new(MockTaskStore::new());
        let config = ProverServiceConfig::default();
        let executor = TaskExecutor::new();

        let handle = ProverServiceBuilder::new(config)
            .with_task_store(store)
            .launch(&executor)
            .await
            .unwrap();

        // Test service operations
    }
}
```

## Error Handling

PaaS provides comprehensive error types:

```rust
use strata_paas::{ProverServiceError, ProverServiceResult};

match result {
    Err(ProverServiceError::TaskNotFound(uuid)) => {
        eprintln!("Task {} not found", uuid);
    }
    Err(ProverServiceError::Internal(e)) => {
        eprintln!("Internal error: {}", e);
    }
    Ok(_) => println!("Success!"),
}
```

## Performance Considerations

### Worker Pool Sizing

- **SP1**: CPU and memory intensive, typically 2-4 workers
- **Risc0**: GPU-friendly, can handle more workers if GPU available
- **Native**: Lightweight, can have many workers

### Retry Strategy

- Use exponential backoff to avoid thundering herd
- Cap max delay to prevent unbounded waiting
- Tune based on typical failure modes (network vs computation)

### Task Store

- Use connection pooling for database task stores
- Consider caching for frequently queried tasks
- Index on UUID for fast lookups

## License

Part of the Strata project.
