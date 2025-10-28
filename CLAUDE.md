# CLAUDE.md - Strata Architecture and Development Documentation

This file provides comprehensive guidance to Claude Code when working with the Strata codebase, including architecture insights, development patterns, and ongoing initiatives.

---

## Table of Contents

1. [Development Commands](#development-commands)
2. [Architecture Overview](#architecture-overview)
3. [Service Framework (`crates/service`)](#service-framework-cratesservice)
4. [Prover Client Refactoring Design](#prover-client-refactoring-design)
5. [Recent Development Activity](#recent-development-activity)
6. [Database Backends](#database-backends)
7. [AI Assistance Disclosure](#ai-assistance-disclosure)

---

## Development Commands

This project uses `just` for task running. Key commands:

### Build & Test
- `just build` - Build the workspace with all features
- `just test-unit` - Run unit tests (requires `cargo-nextest`)
- `just test-int` - Run integration tests
- `just test-functional` - Run functional tests (requires `uv` and `bitcoind`)
- `just test-doc` - Run documentation tests
- `just cov-unit` - Run unit tests with coverage

### Linting & Quality
- `just lint` - Run all linting and formatting checks
- `just lint-fix` - Apply automatic fixes where possible
- `just sec` - Check for security advisories
- `just rustdocs` - Generate documentation

### CI/CD
- `just pr` - Run complete PR checks (lint, docs, unit tests, integration tests, functional tests)
- `just clean` - Clean all build artifacts and test data

### Functional Tests
- `./functional-tests/run_test.sh` - Run all functional tests
- `./functional-tests/run_test.sh -t bridge/bridge_deposit_happy.py` - Run specific test
- `./functional-tests/run_test.sh -g bridge` - Run test group
- `PROVER_TEST=1 ./functional-tests/run_test.sh -g prover` - Run prover tests

---

## Architecture Overview

Alpen is an EVM-compatible validity rollup on Bitcoin. The codebase is organized into the following components:

### Core Components

#### Anchor State Machine (ASM)
- **Location**: `crates/asm/stf/`
- **Purpose**: Core state transition logic that processes Bitcoin blocks and routes transactions to registered subprotocols
- **Entry Point**: `compute_asm_transition()`
- **Architecture**: Uses a subprotocol pattern where different transaction types are routed to specific handlers:
  - `bridge-v1` - Bitcoin bridge functionality
  - `checkpoint-v0` - Checkpoint validation
  - `admin` - Administrative operations
  - `debug-v1` - Debug utilities

#### Consensus Logic (CSM - Chain State Machine)
- **Location**: `crates/consensus-logic/`
- **Purpose**: Handles consensus validation, chain worker context, and sync management
- **Key Components**:
  - Chain State Machine (CSM)
  - Fork choice manager
  - Sync manager
- **Pattern**: CSM listens to ASM logs via the service framework listener pattern

#### Execution Engine Integration
- **Location**: `crates/evmexec/`, `crates/reth/`
- **Purpose**: Interfaces with Reth (Ethereum execution client) for EVM execution
- **Recent Update**: Reth upgraded to 1.8.2 (October 2025)

#### Storage Layer
- **Location**: `crates/db/`, `crates/db-store-*`, `crates/storage/`
- **Purpose**: Multi-backend storage system
- **Backends**: Sled (default), RocksDB (alternative)
- **Pattern**: Database abstractions with backend implementations

### Key Binaries

#### `strata-client` (Main Client)
- **Location**: `bin/strata-client/`
- **Modes**: Sequencer or Full Node
- **Architecture**:
  - Uses `CoreContext` for shared services
  - Spawns multiple critical tasks via `TaskExecutor`
  - Mode-specific initialization (sequencer vs. full node)
  - RPC server with multiple API implementations

#### `alpen-reth`
- **Purpose**: Modified Reth client for Alpen
- **Customizations**: Custom precompiles for bridge and schnorr verification

#### `prover-client`
- **Location**: `bin/prover-client/`
- **Current State**: Standalone binary
- **Purpose**: Zero-knowledge proof generation
- **Target**: Refactor to embeddable library (see [Prover Client Refactoring Design](#prover-client-refactoring-design))

#### `strata-sequencer-client`
- **Location**: `bin/strata-sequencer-client/`
- **Purpose**: Sequencer-specific functionality
- **Pattern**: Duty fetcher + executor pattern

### Bitcoin Integration

#### L1 Integration (`crates/btcio/`)
- **Reader tasks**: Monitor Bitcoin blocks
- **Writer tasks**: Publish transactions
- **Broadcaster**: Transaction propagation
- **Format**: Custom transaction format for embedding Alpen data in Bitcoin transactions

#### L1 Transaction Format
Uses custom format for different operation types:
- Deposit transactions
- Checkpoint transactions
- Withdrawal fulfillment transactions

### Proof System

#### Multiple Proving Backends
- **RISC0**: `provers/risc0/` (legacy, being phased out)
- **SP1**: `provers/sp1/` (primary)

#### Proof Implementations (`crates/proof-impl/`)
- `checkpoint` - Checkpoint proofs
- `cl-stf` - Consensus layer state transition proofs
- `evm-ee-stf` - EVM execution engine state transition proofs

---

## Service Framework (`crates/service`)

The service framework provides structured lifecycle management for worker tasks with status monitoring and graceful shutdown.

### Core Concepts

#### Service Trait
```rust
pub trait Service: Sync + Send + 'static {
    type State: ServiceState;    // In-memory state
    type Msg: ServiceMsg;         // Input message type
    type Status: ServiceStatus;   // Status derived from state

    fn get_status(s: &Self::State) -> Self::Status;
}
```

#### Service Types

1. **AsyncService** - For async/await tasks
   ```rust
   pub trait AsyncService: Service {
       fn on_launch(_state: &mut Self::State) -> impl Future<Output = anyhow::Result<()>>;
       fn process_input(_state: &mut Self::State, _input: &Self::Msg) -> impl Future<Output = anyhow::Result<Response>>;
       fn before_shutdown(_state: &mut Self::State, _err: Option<&anyhow::Error>) -> impl Future<Output = anyhow::Result<()>>;
   }
   ```

2. **SyncService** - For blocking/sync tasks
   ```rust
   pub trait SyncService: Service {
       fn on_launch(_state: &mut Self::State) -> anyhow::Result<()>;
       fn process_input(_state: &mut Self::State, _input: &Self::Msg) -> anyhow::Result<Response>;
       fn before_shutdown(_state: &mut Self::State, _err: Option<&anyhow::Error>) -> anyhow::Result<()>;
   }
   ```

### Service Patterns

#### 1. Command Worker Pattern
**Use Case**: Passive services that respond to explicit commands

```rust
// Define command messages
enum MyCommand {
    DoWork(u32, CommandCompletionSender<String>),
}

// Build and launch service
let mut builder = ServiceBuilder::<MyService, _>::new();
let cmd_handle = builder.create_command_handle(10);
let monitor = builder
    .with_state(state)
    .launch_async("my_service", &executor)
    .await?;

// Use handle to send commands
let result = cmd_handle.send_and_wait(|completion| {
    MyCommand::DoWork(42, completion)
}).await?;
```

**Examples in Codebase**:
- `AsmWorker` - `crates/asm/worker/`
- `ChainWorker` - `crates/chain-worker/`

#### 2. Listener Pattern
**Use Case**: Services that react to status updates from another service

```rust
// Launch monitored service
let monitored_monitor = ServiceBuilder::<MonitoredService, _>::new()
    .with_state(monitored_state)
    .with_input(monitored_input)
    .launch_async("monitored", &executor)
    .await?;

// Create listener that reacts to status changes
let listener_input = StatusMonitorInput::from_receiver(
    monitored_monitor.status_rx.clone()
);

let listener_monitor = ServiceBuilder::<ListenerService, _>::new()
    .with_state(listener_state)
    .with_input(listener_input)
    .launch_async("listener", &executor)
    .await?;
```

**Key Properties**:
- Passive: Only reacts to status updates
- Unaware: Monitored service has no knowledge of listeners
- Coupled lifecycle: Listener exits when monitored service exits
- Own status: Listener maintains its own status structure

**Example**: CSM listens to ASM logs (see `crates/consensus-logic/src/sync_manager.rs`)

### ServiceBuilder Pattern

All services follow this builder pattern:

```rust
pub struct MyWorkerBuilder<W> {
    context: Option<W>,
    params: Option<Arc<Params>>,
    // ... other dependencies
}

impl<W> MyWorkerBuilder<W> {
    pub fn with_context(mut self, context: W) -> Self { ... }
    pub fn with_params(mut self, params: Arc<Params>) -> Self { ... }

    pub fn launch(self, executor: &TaskExecutor) -> Result<MyWorkerHandle>
    where W: WorkerContext + Send + Sync + 'static
    {
        // Validate dependencies
        let context = self.context.ok_or(...)?;

        // Create service state
        let service_state = MyWorkerServiceState::new(context, params);

        // Create service builder
        let mut service_builder =
            ServiceBuilder::<MyWorkerService<W>, _>::new()
                .with_state(service_state);

        // Create command handle
        let command_handle = service_builder.create_command_handle(64);

        // Launch service
        let monitor = service_builder.launch_async("my_worker", executor)?;

        // Return handle
        Ok(MyWorkerHandle::new(command_handle, monitor))
    }
}
```

**Benefits**:
- Encapsulates initialization complexity
- Type-safe dependency injection
- Prevents leaking implementation details
- Returns ergonomic handle for interaction

### Status Monitoring

Services expose status via watch channels:

```rust
pub struct ServiceMonitor<S: ServiceStatus> {
    status_rx: watch::Receiver<S>,
}

impl<S: ServiceStatus> ServiceMonitor<S> {
    pub fn status_rx(&self) -> watch::Receiver<S> { ... }
}
```

Status types must implement:
- `Clone` - For sharing across threads
- `Debug` - For logging
- `Serialize` - For metrics collection
- `Send + Sync + 'static` - For concurrency

---

## Prover Client Refactoring Design

### Current State Analysis

**Location**: `bin/prover-client/`

**Current Architecture** (Standalone Binary):
```
main.rs
  ├── Args (CLI) + Config (TOML)
  ├── ProofOperator (checkpoint, cl_stf, evm_ee operators)
  ├── TaskTracker (Arc<Mutex<...>>)
  ├── ProofDBSled (database)
  ├── ProverManager (spawned task)
  │   └── process_pending_tasks() loop
  │       ├── Fetches pending/retriable tasks
  │       ├── Checks worker limits
  │       └── Spawns make_proof() tasks
  └── CheckpointRunner (optional, spawned task)
      └── checkpoint_proof_runner() loop
  └── RPC Server (ProverClientRpc)
```

**Key Components**:
1. **ProverManager** - Core proving loop that:
   - Polls for pending tasks
   - Manages worker pools (Native/SP1)
   - Spawns proof generation tasks
   - Handles retry logic with exponential backoff

2. **TaskTracker** - State machine for proof tasks:
   - Status: `Pending → ProvingInProgress → Completed/Failed/TransientFailure`
   - Dependency resolution
   - Retry counter management
   - Concurrent access via `Arc<Mutex<...>>`

3. **ProofOperator** - Wrapper around proof implementations:
   - `CheckpointOperator`
   - `ClStfOperator`
   - `EvmEeOperator`

4. **CheckpointRunner** - Autonomous checkpoint proving:
   - Fetches latest unproven checkpoint
   - Creates proof task
   - Submits proof back to sequencer

### Target Architecture (Library)

Create `crates/prover/` library with embeddable components:

```
crates/prover/
├── Cargo.toml
└── src/
    ├── lib.rs              # Public API exports
    ├── config.rs           # Configuration types
    ├── builder.rs          # ProverServiceBuilder
    ├── service.rs          # ProverService (implements AsyncService)
    ├── state.rs            # ProverServiceState
    ├── handle.rs           # ProverHandle
    ├── commands.rs         # Command types
    ├── status.rs           # Status types
    ├── manager/            # Core proving logic
    │   ├── mod.rs
    │   ├── task_tracker.rs
    │   └── worker_pool.rs
    ├── operators/          # Proof generation
    │   ├── mod.rs
    │   ├── operator.rs
    │   ├── checkpoint.rs
    │   ├── cl_stf.rs
    │   └── evm_ee.rs
    └── checkpoint_runner/  # Autonomous checkpoint proving
        ├── mod.rs
        ├── fetch.rs
        └── submit.rs
```

### Architecture Design

#### 1. Service Definition

```rust
// crates/prover/src/service.rs
pub struct ProverService;

impl Service for ProverService {
    type State = ProverServiceState;
    type Msg = ProverCommand;
    type Status = ProverStatus;

    fn get_status(s: &Self::State) -> Self::Status {
        s.generate_status()
    }
}

impl AsyncService for ProverService {
    async fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        // Initialize proving backend
        // Spawn worker pool managers
        // Spawn checkpoint runner (if enabled)
        Ok(())
    }

    async fn process_input(
        state: &mut Self::State,
        cmd: &Self::Msg,
    ) -> anyhow::Result<Response> {
        match cmd {
            ProverCommand::CreateTask { context, deps, completion } => {
                let keys = state.task_tracker.create_tasks(context, deps, &state.db)?;
                completion.send(keys);
            }
            ProverCommand::GetTaskStatus { key, completion } => {
                let status = state.task_tracker.get_task(key)?;
                completion.send(status);
            }
            ProverCommand::GetProof { key, completion } => {
                let proof = state.db.get_proof(key)?;
                completion.send(proof);
            }
            ProverCommand::GetReport { completion } => {
                let report = state.task_tracker.generate_report();
                completion.send(report);
            }
        }
        Ok(Response::Continue)
    }

    async fn before_shutdown(
        state: &mut Self::State,
        _err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        // Gracefully stop worker pools
        // Wait for in-flight proofs (with timeout)
        // Persist task state
        Ok(())
    }
}
```

#### 2. Service State

```rust
// crates/prover/src/state.rs
pub struct ProverServiceState {
    // Configuration
    config: ProverConfig,
    params: Arc<Params>,

    // Core components
    task_tracker: TaskTracker,
    operator: ProofOperator,
    db: Arc<ProofDBSled>,

    // Background task handles
    worker_pool_handles: Vec<JoinHandle<()>>,
    checkpoint_runner_handle: Option<JoinHandle<()>>,

    // RPC clients (if embedded mode)
    el_client: Option<Arc<HttpClient>>,
    cl_client: Option<Arc<HttpClient>>,
    btc_client: Option<Arc<BitcoinClient>>,
}

impl ProverServiceState {
    pub fn new(
        config: ProverConfig,
        params: Arc<Params>,
        db: Arc<ProofDBSled>,
        operator: ProofOperator,
    ) -> Self {
        Self {
            config,
            params,
            task_tracker: TaskTracker::new(),
            operator,
            db,
            worker_pool_handles: Vec::new(),
            checkpoint_runner_handle: None,
            el_client: None,
            cl_client: None,
            btc_client: None,
        }
    }

    pub fn generate_status(&self) -> ProverStatus {
        ProverStatus {
            task_report: self.task_tracker.generate_report(),
            worker_pools: self.config.workers.clone(),
            checkpoint_runner_enabled: self.config.features.enable_checkpoint_runner,
        }
    }
}

impl ServiceState for ProverServiceState {
    fn name(&self) -> &str {
        "prover_service"
    }
}
```

#### 3. Command Messages

```rust
// crates/prover/src/commands.rs
#[derive(Debug)]
pub enum ProverCommand {
    // Task management
    CreateTask {
        context: ProofContext,
        deps: Vec<ProofContext>,
        completion: CommandCompletionSender<Vec<ProofKey>>,
    },
    GetTaskStatus {
        key: ProofKey,
        completion: CommandCompletionSender<ProvingTaskStatus>,
    },

    // Proof retrieval
    GetProof {
        key: ProofKey,
        completion: CommandCompletionSender<Option<ProofReceipt>>,
    },

    // Status/metrics
    GetReport {
        completion: CommandCompletionSender<HashMap<String, usize>>,
    },
}
```

#### 4. Status Types

```rust
// crates/prover/src/status.rs
#[derive(Clone, Debug, Serialize)]
pub struct ProverStatus {
    pub task_report: HashMap<String, usize>,
    pub worker_pools: HashMap<ProofZkVm, usize>,
    pub checkpoint_runner_enabled: bool,
}
```

#### 5. Builder

```rust
// crates/prover/src/builder.rs
pub struct ProverServiceBuilder {
    config: Option<ProverConfig>,
    params: Option<Arc<Params>>,
    db: Option<Arc<ProofDBSled>>,
    el_client: Option<Arc<HttpClient>>,
    cl_client: Option<Arc<HttpClient>>,
    btc_client: Option<Arc<BitcoinClient>>,
}

impl ProverServiceBuilder {
    pub fn new() -> Self {
        Self {
            config: None,
            params: None,
            db: None,
            el_client: None,
            cl_client: None,
            btc_client: None,
        }
    }

    pub fn with_config(mut self, config: ProverConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn with_params(mut self, params: Arc<Params>) -> Self {
        self.params = Some(params);
        self
    }

    pub fn with_database(mut self, db: Arc<ProofDBSled>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn with_rpc_clients(
        mut self,
        el: Arc<HttpClient>,
        cl: Arc<HttpClient>,
        btc: Arc<BitcoinClient>,
    ) -> Self {
        self.el_client = Some(el);
        self.cl_client = Some(cl);
        self.btc_client = Some(btc);
        self
    }

    pub fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<ProverHandle> {
        // Validate dependencies
        let config = self.config.ok_or(ProverError::MissingDependency("config"))?;
        let params = self.params.ok_or(ProverError::MissingDependency("params"))?;
        let db = self.db.ok_or(ProverError::MissingDependency("db"))?;

        // Create operator
        let operator = ProofOperator::init(
            self.btc_client.ok_or(...)?,
            self.el_client.ok_or(...)?,
            self.cl_client.ok_or(...)?,
            params.clone(),
            config.features.enable_checkpoint_runner,
        );

        // Create service state
        let state = ProverServiceState::new(config, params, db, Arc::new(operator));

        // Create service builder
        let mut service_builder =
            ServiceBuilder::<ProverService, _>::new()
                .with_state(state);

        // Create command handle
        let command_handle = service_builder.create_command_handle(100);

        // Launch service
        let monitor = service_builder.launch_async("prover", executor)?;

        // Return handle
        Ok(ProverHandle::new(command_handle, monitor))
    }
}
```

#### 6. Handle

```rust
// crates/prover/src/handle.rs
#[derive(Clone)]
pub struct ProverHandle {
    command_handle: CommandHandle<ProverCommand>,
    monitor: ServiceMonitor<ProverStatus>,
}

impl ProverHandle {
    pub fn new(
        command_handle: CommandHandle<ProverCommand>,
        monitor: ServiceMonitor<ProverStatus>,
    ) -> Self {
        Self {
            command_handle,
            monitor,
        }
    }

    // High-level API methods
    pub async fn create_task(
        &self,
        context: ProofContext,
        deps: Vec<ProofContext>,
    ) -> anyhow::Result<Vec<ProofKey>> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::CreateTask {
                context,
                deps,
                completion,
            })
            .await
    }

    pub async fn get_task_status(
        &self,
        key: ProofKey,
    ) -> anyhow::Result<ProvingTaskStatus> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetTaskStatus {
                key,
                completion,
            })
            .await
    }

    pub async fn get_proof(
        &self,
        key: ProofKey,
    ) -> anyhow::Result<Option<ProofReceipt>> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetProof {
                key,
                completion,
            })
            .await
    }

    pub async fn get_report(&self) -> anyhow::Result<HashMap<String, usize>> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetReport {
                completion,
            })
            .await
    }

    pub fn status_rx(&self) -> watch::Receiver<ProverStatus> {
        self.monitor.status_rx()
    }
}
```

### Integration Patterns

#### Pattern 1: Embedded in `strata-client` (Sequencer Mode)

```rust
// bin/strata-client/src/main.rs

fn start_sequencer_tasks(
    ctx: CoreContext,
    config: &Config,
    executor: &TaskExecutor,
    // ... other params
) -> anyhow::Result<()> {
    // ... existing sequencer setup

    // Add prover service
    if config.sequencer.enable_embedded_prover {
        let prover_config = ProverConfig::from_file(&config.sequencer.prover_config_path)?;
        let prover_db = init_prover_database(&config.sequencer.prover_datadir)?;

        let prover_handle = ProverServiceBuilder::new()
            .with_config(prover_config)
            .with_params(ctx.params.clone())
            .with_database(prover_db)
            .with_rpc_clients(
                ctx.engine.clone(),
                Arc::new(create_cl_client(&config)?),
                ctx.bitcoin_client.clone(),
            )
            .launch(executor)?;

        // Store handle in context for RPC access
        ctx.prover_handle = Some(prover_handle);
    }

    // ... rest of sequencer initialization
}
```

#### Pattern 2: Embedded in `strata-sequencer-client`

```rust
// bin/strata-sequencer-client/src/main.rs

async fn main_inner(args: Args) -> anyhow::Result<()> {
    // ... existing setup

    // Launch prover service
    let prover_handle = ProverServiceBuilder::new()
        .with_config(config.prover)
        .with_params(params)
        .with_database(prover_db)
        .with_rpc_clients(el_client, cl_client, btc_client)
        .launch(&executor)?;

    // Use prover handle in duty executor
    let duty_executor = DutyExecutor::new(prover_handle.clone());

    // ... rest of initialization
}
```

#### Pattern 3: Standalone Binary (Backward Compatibility)

```rust
// bin/prover-client/src/main.rs

async fn main_inner(args: Args) -> anyhow::Result<()> {
    // Initialize logging, config, etc.
    let config = args.resolve_config()?;
    let params = args.resolve_and_validate_rollup_params()?;

    // Setup clients and database
    let el_client = HttpClientBuilder::default()
        .build(config.get_reth_rpc_url())?;
    let cl_client = HttpClientBuilder::default()
        .build(config.get_sequencer_rpc_url())?;
    let btc_client = create_bitcoin_client(&config)?;
    let prover_db = open_prover_database(&config.datadir)?;

    // Launch prover service using new library
    let prover_handle = ProverServiceBuilder::new()
        .with_config(config.prover)
        .with_params(Arc::new(params))
        .with_database(prover_db)
        .with_rpc_clients(
            Arc::new(el_client),
            Arc::new(cl_client),
            Arc::new(btc_client),
        )
        .launch(&executor)?;

    // Start RPC server with handle
    let rpc_server = ProverClientRpc::new(prover_handle.clone());
    rpc_server
        .start_server(config.get_dev_rpc_url(), config.enable_dev_rpcs)
        .await?;

    Ok(())
}
```

### Migration Strategy

#### Phase 1: Create Library Structure
1. Create `crates/prover/` directory
2. Move shared types and logic from `bin/prover-client/src/`
3. Keep binary intact, update imports
4. Ensure all tests pass

#### Phase 2: Implement Service Pattern
1. Implement `ProverService` using service framework
2. Create `ProverServiceBuilder` and `ProverHandle`
3. Add comprehensive unit tests
4. Update binary to use service pattern

#### Phase 3: Integration
1. Add embedded prover support to `strata-client`
2. Add embedded prover support to `strata-sequencer-client`
3. Update configuration types
4. Add integration tests

#### Phase 4: Optimization
1. Add metrics/observability
2. Optimize worker pool management
3. Improve error handling and retry logic
4. Performance testing

### Configuration Changes

Add to `strata-config`:

```rust
// crates/config/src/lib.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencerConfig {
    // ... existing fields

    /// Enable embedded prover
    #[serde(default)]
    pub enable_embedded_prover: bool,

    /// Path to prover configuration file
    #[serde(default)]
    pub prover_config_path: Option<PathBuf>,

    /// Prover data directory
    #[serde(default)]
    pub prover_datadir: Option<PathBuf>,
}
```

### Benefits of This Architecture

1. **Reusability**: Prover logic can be embedded in any binary
2. **Lifecycle Management**: Automatic via service framework
3. **Status Monitoring**: Built-in via watch channels
4. **Graceful Shutdown**: Handled by service framework
5. **Type Safety**: Strong typing for commands and responses
6. **Testability**: Easy to mock CommandHandle for testing
7. **Backward Compatibility**: Standalone binary still works
8. **Clean Separation**: Clear API boundaries via handle

### Testing Strategy

```rust
// crates/prover/tests/integration_test.rs

#[tokio::test]
async fn test_prover_service_basic() {
    let task_manager = TaskManager::new_test();
    let executor = task_manager.executor();

    let config = ProverConfig::default();
    let params = Arc::new(test_params());
    let db = Arc::new(test_db());

    let handle = ProverServiceBuilder::new()
        .with_config(config)
        .with_params(params)
        .with_database(db)
        .with_rpc_clients(mock_el(), mock_cl(), mock_btc())
        .launch(&executor)
        .unwrap();

    // Create a proof task
    let keys = handle.create_task(
        ProofContext::Checkpoint(0),
        vec![],
    ).await.unwrap();

    // Poll for completion
    for _ in 0..100 {
        let status = handle.get_task_status(keys[0]).await.unwrap();
        if matches!(status, ProvingTaskStatus::Completed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Verify proof exists
    let proof = handle.get_proof(keys[0]).await.unwrap();
    assert!(proof.is_some());
}
```

---

## Recent Development Activity

### Last Two Weeks Summary (October 14-28, 2025)

**88 commits** from **10 contributors**

#### Major Initiatives

##### 1. Architecture Refactoring & Code Consolidation
**PR #1100** - Massive 134-file refactor
- Consolidated operator types into new `strata-params` crate
- Replaced `RollupVerifyingKey` with `PredicateKey` throughout codebase
- Removed 852 lines, added 429 lines (net -423 lines)
- Eliminated duplicate code from `primitives/params.rs`
- Updated zkaleido to rc17
- **Affected**: ASM subprotocols, consensus logic, bridge types, proof system

**Impact**: Cleaner dependency graph, better type organization, easier maintenance

##### 2. Reth Upgrade to 1.8.2
**PR #1034** - Critical dependency update
- Bumped Reth and related dependencies to 1.8.2
- Refactored precompile IDs for bridge and schnorr signature verification
- Updated RPC implementations (eth call, pending block, transactions)
- Modified engine and node implementations
- Updated all SP1 guest program locks

**Impact**: Keeps Alpen in sync with upstream Ethereum execution improvements

##### 3. Database Tooling Enhancements
**PRs #1108, #1103, #1063** - Heavy dbtool focus

Key improvements:
- Dry-run mode for `revert-chainstate` command (safer operations)
- Fixed epoch summary cleanup logic
- Better L1 writer DB delete API for non-unique intent IDs
- Improved checkpoint, epoch summary, and L1 entry deletion logic
- Extensive functional test refactoring - added `utils/dbtool.py` helper
- New test: `revert_chainstate_dry_run.py`

**Impact**: Safer chainstate management, better operational tooling

#### Active Development Branches

##### SSZ Serialization Migration
**Branches**: `ssz-1`, `ssz-2`, `ssz-3`, `ssz-borsh-checkpoint-types`

Major ongoing work (not yet merged):
- Creating SSZ-specific type crates:
  - `acct-ssz-types`
  - `ee-chain-ssz-types`
  - `checkpoint-types-ssz` (SPS-62 spec)
- SSZ vs Borsh benchmarking
- Converting account identifiers, BitcoinAmount, checkpoint types to SSZ
- Code generation for SSZ types

**Strategic Direction**: Moving from Borsh to SSZ serialization (Ethereum standard) for better interoperability

##### Optimistic Layer (OL) Foundation
**PR #1085** - New subsystem

Created foundational types:
- `crates/ol/chain-types`: Block, Transaction, Log types
- `crates/ol/state-types`: State types
- Total: 627 new lines for OL infrastructure

**Strategic Direction**: Building parallel optimistic rollup layer alongside validity rollup

#### Development Themes

1. **Quality & Safety Focus**
   - Re-enabling previously disabled bridge tests
   - Dry-run modes for dangerous operations
   - Extensive functional test improvements
   - Dependabot integration with cooldowns

2. **Code Health**
   - Naming convention enforcement
   - Documentation improvements (ASM/CSM clarification)
   - TOML formatting fixes
   - Reducing code duplication

3. **Modularization**
   - Moving types to dedicated crates
   - Better separation of concerns
   - Cleaner dependency boundaries

---

## Database Backends

The project supports two database backends (configure via Cargo features):
- `sled` - Default, embedded database
- `rocksdb` - Alternative backend for performance

**Important**: Cannot enable both backends simultaneously in release builds.

---

## Configuration

Configuration uses TOML format. See `example_config.toml` for reference:
- Client settings (RPC port, data directory, sequencer mode)
- Bitcoin RPC connection
- Execution engine (Reth) connection
- Sync parameters and L1 follow distance

---

## Development Prerequisites

Install required tools:
- `just` - Task runner
- `cargo-nextest` - Modern test runner
- `cargo-audit` - Security vulnerability scanner
- `taplo` - TOML formatter/linter
- `codespell` - Spell checker
- `bitcoind` - Required for functional tests
- `uv` - Python package manager for functional tests

**Alternative**: Use Nix development shell with `nix develop`.

---

## AI Assistance Disclosure

When contributing with AI assistance, disclose in pull requests per `CONTRIBUTING.md` requirements.

---

**Last Updated**: October 28, 2025
**Document Version**: 1.0.0
