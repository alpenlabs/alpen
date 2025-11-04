# PaaS (Prover-as-a-Service) Documentation

**Last Updated:** 2025-11-04

This directory contains comprehensive documentation for the Strata PaaS (Prover-as-a-Service) implementation.

## Overview

PaaS is an embeddable proof generation service that follows Strata's command worker pattern from `crates/service`. It manages the lifecycle of zero-knowledge proof generation tasks with proper state machine semantics, retry logic, and worker pool management.

## Documentation Structure

- **[DESIGN.md](./DESIGN.md)** - Architecture, design decisions, and system overview
- **[TESTING.md](./TESTING.md)** - Testing strategies, test coverage, and verification procedures
- **[TROUBLESHOOTING.md](./TROUBLESHOOTING.md)** - Common issues, debugging tips, and solutions
- **[DEVELOPMENT.md](./DEVELOPMENT.md)** - Development workflow, code organization, and contribution guidelines
- **[CHANGELOG.md](./CHANGELOG.md)** - Detailed history of changes and milestones

## Quick Start

### Basic Usage

```rust
use strata_paas::{PaaSConfig, ProverBuilder};
use strata_primitives::proof::ProofContext;

// Launch the prover service
let handle = ProverBuilder::new()
    .with_config(config)
    .with_proof_operator(operator)
    .with_database(database)
    .launch(&executor)?;

// Submit a proof task
let task_id = handle.create_task(
    ProofContext::Checkpoint { index: 42 },
    vec![], // no dependencies
).await?;

// Check status
let status = handle.get_task_status(task_id).await?;

// Retrieve completed proof
let proof = handle.get_proof(task_id).await?;
```

## Key Features

- **State Machine Correctness** - Enforces proper task transitions: `Pending → Queued → Proving → Completed`
- **Retry Logic** - Automatic retry with exponential backoff for transient failures
- **Worker Pool Management** - Configurable worker pools per proving backend (Native/SP1)
- **Dependency Resolution** - Supports proof tasks with dependencies on other proofs
- **Command Pattern** - Clean async API via ProverHandle
- **Status Monitoring** - Real-time status updates via watch channels

## Current Status

**Version:** 0.1.0 (Phase 2 Complete)
**Test Coverage:** 13/16 prover functional tests passing (81%)
**Production Readiness:** ✅ Operational

## Recent Updates

- **2025-11-04**: Fixed critical state machine bug, improved from 12.5% to 81% test pass rate
- **2025-11-04**: Resolved TODOs, improved documentation, zero clippy warnings
- **2025-10-30**: Completed Phase 2 migration from standalone binary to embeddable library

## Getting Help

1. Check [TROUBLESHOOTING.md](./TROUBLESHOOTING.md) for common issues
2. Review service logs at `functional-tests/_dd/<test-id>/prover/prover_client/service.log`
3. Use `ProverHandle::get_report()` for runtime metrics

## Contributing

See [DEVELOPMENT.md](./DEVELOPMENT.md) for development workflow and code organization.
