# PaaS Development Guide

**Last Updated:** 2025-11-04

## Table of Contents

1. [Development Setup](#development-setup)
2. [Code Organization](#code-organization)
3. [Development Workflow](#development-workflow)
4. [Coding Standards](#coding-standards)
5. [Adding Features](#adding-features)
6. [Debugging Guide](#debugging-guide)

---

## Development Setup

### Prerequisites

```bash
# Rust toolchain
rustup update stable

# Development tools
cargo install cargo-nextest
cargo install cargo-audit
cargo install taplo-cli

# For functional tests
# - Python 3.x with uv
# - bitcoind

# Optional: Nix
nix develop
```

### Building

```bash
# Build PaaS crate
cargo build -p strata-paas

# Build with all features
cargo build -p strata-paas --all-features

# Build prover-client (uses PaaS)
cd bin/prover-client && cargo build
```

### Running Tests

```bash
# Unit tests
cargo nextest run -p strata-paas

# Clippy
cargo clippy -p strata-paas --all-targets -- -D warnings

# Format
cargo fmt --check

# Full lint suite
just lint
```

---

## Code Organization

### Directory Structure

```
crates/paas/
├── src/
│   ├── lib.rs              # Public API exports, module documentation
│   ├── builder.rs          # ProverBuilder (fluent API)
│   ├── commands.rs         # PaaSCommand enum, request/response types
│   ├── config.rs           # Configuration types
│   ├── errors.rs           # Error types
│   ├── handle.rs           # ProverHandle (public API)
│   ├── service.rs          # ProverService (AsyncService impl)
│   ├── state.rs            # ProverServiceState
│   ├── status.rs           # Status types
│   └── manager/
│       ├── mod.rs          # Manager module exports
│       ├── task_tracker.rs # State machine implementation
│       └── worker_pool.rs  # Worker pool implementation
├── docs/                   # Documentation (this directory)
│   ├── README.md
│   ├── DESIGN.md
│   ├── TESTING.md
│   ├── TROUBLESHOOTING.md
│   ├── DEVELOPMENT.md
│   └── CHANGELOG.md
├── Cargo.toml
└── README.md (symlink to docs/README.md)
```

### Module Responsibilities

#### Public API (`lib.rs`)

Exports:
- `ProverBuilder` - Fluent API for service construction
- `ProverHandle` - Command-based client API
- `ProverService` - Service implementation (for advanced use)
- Configuration types: `PaaSConfig`, `WorkerConfig`, `RetryConfig`
- Status types: `PaaSStatus`, `PaaSReport`, `TaskStatus`
- Error types: `PaaSError`

#### Commands (`commands.rs`)

Defines:
- `PaaSCommand` enum - All service command variants
- Request/response types
- `TaskId`, `ProofData` types
- `CommandCompletionSender` aliases

#### Service State (`state.rs`)

Manages:
- TaskTracker instance
- ProofOperator instance
- ProofDatabase instance
- Configuration
- Cumulative statistics
- State transition methods

#### Task Tracker (`manager/task_tracker.rs`)

Implements:
- State machine logic
- Task metadata storage
- Dependency resolution
- Retry tracking
- Valid transition enforcement

**Critical Code:**
```rust
// Lines 54-82: State transition validation
impl InternalTaskStatus {
    fn transition(&mut self, target: InternalTaskStatus) -> Result<(), PaaSError> {
        let is_valid = match (self.clone(), &target) {
            (_, InternalTaskStatus::Failed) => true,
            (InternalTaskStatus::Pending, InternalTaskStatus::Queued) => true,
            (InternalTaskStatus::Queued, InternalTaskStatus::Proving) => true,
            (InternalTaskStatus::Proving, InternalTaskStatus::Completed) => true,
            // ... more transitions
            _ => false,
        };
        // ...
    }
}
```

#### Worker Pool (`manager/worker_pool.rs`)

Implements:
- Polling loop for pending/retriable tasks
- Worker limit enforcement
- Async task spawning
- State transition orchestration

**Critical Code:**
```rust
// Lines 129-169: Proof generation task lifecycle
spawn(async move {
    info!(?task_id, ?proof_key, "Starting proof generation");

    // State transitions
    if let Err(e) = prover_handle.mark_queued(task_id).await {
        error!(?task_id, ?e, "Failed to mark task as queued");
        return;
    }

    if let Err(e) = prover_handle.mark_proving(task_id).await {
        error!(?task_id, ?e, "Failed to mark task as proving");
        return;
    }

    // Actual proof generation
    let result = operator.process_proof(proof_key, database.as_ref()).await;

    // Handle result
    match result {
        Ok(()) => {
            info!(?task_id, "Proof generation completed");
            if let Err(e) = prover_handle.mark_completed(task_id).await {
                error!(?task_id, ?e, "Failed to mark task as completed");
            }
        }
        Err(e) => {
            let error_msg = e.to_string();
            error!(?task_id, ?error_msg, "Proof generation failed");
            if let Err(e) = prover_handle
                .mark_transient_failure(task_id, error_msg)
                .await
            {
                error!(?task_id, ?e, "Failed to mark task as failed");
            }
        }
    }
});
```

---

## Development Workflow

### 1. Starting Development

```bash
# Create feature branch
git checkout -b feature/my-feature

# Make changes
$EDITOR crates/paas/src/...

# Run tests frequently
cargo nextest run -p strata-paas

# Check formatting
cargo fmt

# Check lints
cargo clippy -p strata-paas --all-targets -- -D warnings
```

### 2. Making Changes

#### Adding New Command

1. **Add command variant** (`commands.rs`):
```rust
pub enum PaaSCommand {
    // ... existing commands

    /// New command description
    MyNewCommand {
        /// Parameter description
        param: Type,
        /// Completion channel
        completion: CommandCompletionSender<Result<ResponseType, PaaSError>>,
    },
}
```

2. **Add handle method** (`handle.rs`):
```rust
impl ProverHandle {
    /// My new command
    pub async fn my_new_command(&self, param: Type) -> Result<ResponseType, PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::MyNewCommand {
                param,
                completion,
            })
            .await
            .map_err(convert_service_error)?
    }
}
```

3. **Add service handler** (`service.rs`):
```rust
impl AsyncService for ProverService<D> {
    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            // ... existing handlers

            PaaSCommand::MyNewCommand { param, completion } => {
                let result = state.handle_my_command(*param);
                completion.send(result).await;
                Ok(Response::Continue)
            }
        }
    }
}
```

4. **Add state method** (`state.rs`):
```rust
impl<D: ProofDatabase> ProverServiceState<D> {
    pub fn handle_my_command(&mut self, param: Type) -> Result<ResponseType, PaaSError> {
        // Implementation
    }
}
```

5. **Add tests**

6. **Update documentation**

#### Adding New State

1. **Update TaskTracker** (`manager/task_tracker.rs`):
```rust
pub enum InternalTaskStatus {
    // ... existing states
    MyNewState,
}

impl InternalTaskStatus {
    fn transition(&mut self, target: InternalTaskStatus) -> Result<(), PaaSError> {
        let is_valid = match (self.clone(), &target) {
            // ... existing transitions
            (InternalTaskStatus::SomeState, InternalTaskStatus::MyNewState) => true,
            _ => false,
        };
        // ...
    }
}
```

2. **Add transition methods in `state.rs`**

3. **Update documentation diagrams**

4. **Add comprehensive tests**

### 3. Testing Changes

```bash
# Unit tests
cargo nextest run -p strata-paas

# Integration with prover-client
cd bin/prover-client && cargo test

# Functional tests (if state machine changed)
cd functional-tests && PROVER_TEST=1 ./run_test.sh -g prover

# Specific test
cd functional-tests && ./run_test.sh -t prover/prover_el_deposit_withdraw.py
```

### 4. Pre-commit Checklist

- [ ] Code formatted: `cargo fmt`
- [ ] No clippy warnings: `cargo clippy -p strata-paas -- -D warnings`
- [ ] Unit tests pass: `cargo nextest run -p strata-paas`
- [ ] Documentation updated
- [ ] Changelog updated (if significant change)
- [ ] No debug prints or commented code

### 5. Creating PR

```bash
# Run full PR checks
just pr

# If all pass, create commit
git add -A
git commit -m "feat(paas): Add new feature

- Describe what was added
- Explain why
- Note any breaking changes

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"

# Push and create PR
git push origin feature/my-feature
```

---

## Coding Standards

### Rust Style

Follow standard Rust conventions:
- Use `rustfmt` default configuration
- Follow Rust API guidelines
- Use descriptive variable names
- Add documentation comments for public items

### Documentation

**Module-level docs:**
```rust
//! Brief module description
//!
//! Detailed explanation of module purpose and architecture.
//!
//! # Example
//!
//! ```no_run
//! // Usage example
//! ```
```

**Function docs:**
```rust
/// Brief function description
///
/// More detailed explanation if needed.
///
/// # Arguments
///
/// * `param` - Parameter description
///
/// # Returns
///
/// Return value description
///
/// # Errors
///
/// Error conditions
pub fn my_function(param: Type) -> Result<Output, Error> {
    // ...
}
```

### Error Handling

Use descriptive error messages:
```rust
// ✅ GOOD
Err(PaaSError::InvalidTransition {
    from: current_state,
    to: target_state,
})

// ❌ BAD
Err(PaaSError::Failed)
```

### Logging

Use appropriate log levels:
```rust
// ERROR - Critical failures that prevent operation
error!(?task_id, ?error, "Failed to process task");

// WARN - Issues that may cause problems but aren't critical
warn!(?task_id, "Task retry attempt {}", retry_count);

// INFO - Important events in normal operation
info!(?task_id, "Task completed successfully");

// DEBUG - Detailed execution information
debug!(?task_id, ?state, "State transition");

// TRACE - Very detailed debugging (rarely used)
trace!("Entering function with params: {:?}", params);
```

### State Machine Changes

When modifying state machine:

1. **Document all transitions**
2. **Update DESIGN.md diagrams**
3. **Add comprehensive tests**
4. **Verify no regressions in functional tests**
5. **Update TROUBLESHOOTING.md** with new error patterns

**Critical:**  Always ensure state transitions are valid and enforceable.

---

## Adding Features

### Feature Checklist

When adding a significant feature:

- [ ] Design documented in DESIGN.md
- [ ] API added to ProverHandle
- [ ] Command added to PaaSCommand
- [ ] Service handler implemented
- [ ] State method implemented
- [ ] Unit tests added
- [ ] Integration tested with prover-client
- [ ] Functional tests updated/added
- [ ] Documentation updated
- [ ] Changelog entry added
- [ ] Performance impact considered
- [ ] Error handling comprehensive

### Example: Adding Task Cancellation

1. **Design Phase**
   - Document cancellation semantics
   - Define state transitions (Proving → Cancelled)
   - Plan worker coordination

2. **Implementation Phase**
   - Add `Cancelled` state to TaskTracker
   - Add `cancel_task()` method to state
   - Add `CancelTask` command
   - Implement worker interruption

3. **Testing Phase**
   - Test cancellation of pending task
   - Test cancellation of in-flight task
   - Test double-cancellation handling
   - Test cancellation with dependencies

4. **Documentation Phase**
   - Update state machine diagram
   - Add cancellation examples
   - Document edge cases
   - Add troubleshooting section

---

## Debugging Guide

### Debugging Tests

```bash
# Run single test with output
cargo nextest run -p strata-paas my_test -- --nocapture

# Run with increased logging
RUST_LOG=debug cargo nextest run -p strata-paas

# Run with backtraces
RUST_BACKTRACE=1 cargo nextest run -p strata-paas
```

### Debugging Functional Tests

```bash
# Run test with detailed logs
cd functional-tests
./run_test.sh -t prover/prover_checkpoint_manual.py

# Check service logs
cat _dd/<test-id>/prover/prover_client/service.log

# Analyze specific task
grep "task_id=<id>" _dd/<test-id>/prover/prover_client/service.log
```

### Using GDB/LLDB

```bash
# Build with debug symbols
cargo build -p prover-client

# Run under debugger
rust-gdb target/debug/prover-client

# Set breakpoints
(gdb) break strata_paas::manager::worker_pool::WorkerPool::run
(gdb) run

# Inspect state
(gdb) print task_tracker
```

### Common Debugging Scenarios

#### Debugging State Machine Issues

1. **Add debug logging**:
```rust
debug!(?task_id, ?current_state, ?target_state, "Attempting transition");
```

2. **Check transition validity**:
```rust
if let Err(e) = self.transition(target) {
    error!(?self, ?target, ?e, "Invalid transition attempted");
    return Err(e);
}
```

3. **Trace state history**:
```rust
// Add to TaskMetadata
state_history: Vec<(InternalTaskStatus, Instant)>,
```

#### Debugging Worker Pool Issues

1. **Log worker activity**:
```rust
debug!(?pending_count, ?in_progress, ?worker_limit, "Worker pool iteration");
```

2. **Track task assignment**:
```rust
info!(?task_id, ?backend, worker_index, "Assigning task to worker");
```

3. **Monitor worker limits**:
```rust
if in_progress >= total_workers {
    warn!(?backend, ?in_progress, ?total_workers, "Worker limit reached");
}
```

---

## Tools and Utilities

### Log Analysis

**Grep patterns:**
```bash
# Find all state transitions
grep "marked as" service.log

# Find errors
grep -E "(ERROR|WARN)" service.log

# Find specific task lifecycle
grep "task_id=abc123" service.log | less
```

**Custom analysis scripts:**
- `/tmp/analyze_prover_timing.sh` - Task duration analysis
- `/tmp/prover_health_report.md` - Service health report

### Performance Profiling

```bash
# CPU profiling
cargo build --release
perf record -g target/release/prover-client
perf report

# Memory profiling
valgrind --tool=massif target/release/prover-client
```

### Benchmarking

```bash
# Criterion benchmarks (if added)
cargo bench -p strata-paas

# Custom timing
use std::time::Instant;
let start = Instant::now();
// ... operation
println!("Duration: {:?}", start.elapsed());
```

---

## Contributing Guidelines

1. **Follow the development workflow**
2. **Write comprehensive tests**
3. **Document all public APIs**
4. **Keep commits atomic and well-described**
5. **Update relevant documentation**
6. **Run full PR checks before submitting**

See also:
- [DESIGN.md](./DESIGN.md) - For architecture understanding
- [TESTING.md](./TESTING.md) - For test strategies
- [TROUBLESHOOTING.md](./TROUBLESHOOTING.md) - For debugging help
