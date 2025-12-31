# Observability Strategy & Action Items

> **📋 MANDATORY STANDARDS:** See [`guidelines.md`](./guidelines.md) for the mandatory observability requirements that ALL new code must follow. These guidelines prevent the issues documented below from recurring.

---

## Why Observability Matters

Working on infrastructure, testing, and engineering has led to a key insight: **The resilience of a system relies on observability of the system.**

Observability is being able to know what went wrong when something goes wrong. It's being able to ask questions to the system. With proper observability, a system develops an ability to digest errors.

**Recent example:** Cloudflare's November 18, 2025 outage was caused by an unwrap. [Blog post](https://blog.cloudflare.com/18-november-2025-outage/)

As we plan for mainnet, we don't want to spend hours trying to reconstruct what actually went wrong when something goes wrong.

---

## The Problem: Log Soup

### What We Have
- Basic tracing-subscriber with EnvFilter in `crates/common/src/logging.rs`
- Optional OpenTelemetry output (via STRATA_OTLP_URL env var)
- Logs that are more like `print` style debugging
- Combination of asynchronous actors (services) with message passing
- No spans used anywhere
- No consistent field naming conventions

### The Log Soup We're Drowning In

#### Problem 1: No Correlation Between Events
Due to the nature of async code, there's no correlation between log events:

```
07:13:37.782  INFO  ASM found pivot anchor state pivot_block=100
07:13:37.801  INFO  Successfully reassigned expired assignments
07:13:44.958  WARN  tripped bail interrupt, exiting ctx=duty_sign_block
```

The first line is about ASM, the second about duty assignments, the third about block signing. They're interleaved randomly.

**Without explicit correlation, we can't trace one operation end-to-end.**

---

#### Problem 2: Inconsistent Block ID Formatting
We have **three different versions** of how we log block IDs:

```rust
// Version 1: blkid
INFO strata_btcio::reader::query: accepted new block fetch_height=222 blkid=0597a71824abfc073c848664a1108a2bde5043be4a2f46b6d65b5499e04622bb

// Version 2: l1blockid
INFO strata_btcio::reader::handler: wrote L1 block manifest height=100 l1blockid=347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30 txs=0

// Version 3: block_id with abbreviated value in message
INFO handlemsg: strata_asm_worker::service: ASM transition success block_id=101@8313..da31 service=asm_worker input=L1BlockCommitment(height=101, blkid=31da6bb9d50589812a7afcb6800240058057f53b72cf98b9579e035f1f041383)
```

**Can't grep for one block across all logs.**

---

#### Problem 3: Abbreviated Identifiers Break Grep
Some places we abbreviate identifiers:

```
INFO  processing block blkid=aa026e..91422d
```

**If we grep for the full block ID, we miss these crucial events.**

---

#### Problem 4: Logical Architecture ≠ Code Structure

Code structure defines where our code lives. Logical architecture reflects what the system does.

Example: `generate_block_template` is **ONE logical operation** that crosses **FOUR crates**:

```
strata_sequencer::block_template::prepare_block
    ↓ (prepares L1 segment)
strata_consensus_logic::checkpoint_verification::verify_proof
    ↓ (verifies checkpoint proofs)
strata_eectl::engine::prepare_payload
    ↓ (executes EVM transactions)
strata_chaintsn::transition::process_block
    ↓ (computes state transition)
Back to strata_sequencer::block_template (final assembly)
```

**Without component spans, logs look like independent activities:**

```
2025-12-04T10:20:15.123Z INFO strata_sequencer::block_template::block_assembly: preparing block
2025-12-04T10:20:15.201Z DEBUG strata_consensus_logic::checkpoint_verification: verifying checkpoint proof
2025-12-04T10:20:15.356Z INFO strata_eectl::engine: preparing EVM payload
2025-12-04T10:20:15.892Z DEBUG strata_eectl::engine: payload execution complete
2025-12-04T10:20:15.934Z DEBUG strata_chaintsn::transition: processing state transition
2025-12-04T10:20:15.987Z INFO strata_sequencer::block_template::block_assembly: block prepared
```

How do we know these six log lines are part of the same block assembly? We don't.

**We need explicit correlation to trace operations across crate boundaries.**

---

## What We Need: Explicit Correlation

Looking at the entire lifecycle of events that led to a particular case of concern is hard.

**We need to be able to:**
1. Pick any interesting state or log line
2. See its complete lifecycle from entry to exit
3. Trace across async boundaries
4. Reconstruct events in chronological order when needed

**This requires using spans, not just events.**

---

## Core Concepts (from tracing crate)

### Span
A **span** represents a _period of time_ with a beginning and an end. Unlike a log line that represents a _moment in time_, a span tracks execution through a context.

When a program begins executing in a context, it _enters_ the span. When it stops, it _exits_ the span.

### Event
An **event** represents a _moment_ in time. It's comparable to a log line but can occur within the context of a span.

### Subscriber
**Subscribers** are notified when events take place and when spans are entered or exited. This is the power of tracing over traditional logging:

- **Logs** → Fixed format, goes to stdout/file
- **Tracing** → Events + Spans → Subscribers → Multiple outputs (logs, metrics, distributed traces, custom logic)

**One instrumentation → Many consumers.**

Reference: [OpenTelemetry Specification Principles](https://opentelemetry.io/docs/specs/otel/specification-principles/)

---

## Our Goals

### Filtering Goals
We should be able to filter logs by:
- **Component/subsystem** (ASM worker, fork choice, block template)
- **Operation type** (block processing, RPC call, state transition)
- **Entity** (specific block, transaction, epoch)
- **Status** (success, error, timeout)

**Without knowing file paths or function names.**

### Performance Visibility
- How long each operation took
- Where time was spent (network, CPU, database)
- Which operations are slow outliers
- Trends over time

**Without manual instrumentation or profiling.**

### Context on Error
When something fails, automatically get:
- What operation was being attempted
- What entity was being processed
- What the system state was
- What the error chain was
- Who/what triggered the operation

**Without manual error message crafting.**

---

## The Gap

Documentation exists (`improvements.md`, `standards-to-follow.md`) but:
- No infrastructure to make it easy
- No enforced conventions
- No spans used in actual code
- Team doesn't know where to start

**This document bridges that gap.**

---

## Observability Strategy Overview

### Phase 1: Foundation (Week 1-2)
- Fix abbreviated IDs (critical for grep)
- Add field name constants module (code enforcement)
- Document component naming standards
- Add automatic performance timing to logger (duration visibility)

### Phase 2: Core Instrumentation (Week 3-4)
- Instrument L1 pipeline with spans (ASM → CSM → Consensus)
- Add component tags everywhere
- Convert manual field duplication to span inheritance
- Add rate limiting to hot loops

### Phase 3: Advanced Features (Week 5-6)
- Cross-service RPC tracing
- Performance metrics extraction
- Grafana/Loki integration
- Custom subscribers for business logic

---

## Infrastructure Changes Needed

### 1. Extend `crates/common/src/logging.rs`

**Current code (lines 42-83):**
```rust
pub fn init(config: LoggerConfig) {
    let filt = tracing_subscriber::EnvFilter::from_default_env();
    let stdout_sub = tracing_subscriber::fmt::layer().compact().with_filter(filt);
    // ...
}
```

**What needs to be added:**

#### A. Performance Visibility - Automatic Span Timing

**Goal:** Enable visibility into **how long each operation took**, **where time was spent**, and **trends over time**.

**Current problem:**
```rust
// Current logging.rs (line 50):
let stdout_sub = tracing_subscriber::fmt::layer().compact().with_filter(filt);
```

This doesn't show:
- When spans start/end
- How long operations take
- Where bottlenecks are

**Solution:** Add automatic span timing and duration tracking.

**Changes to `crates/common/src/logging.rs`:**

```rust
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Debug, Clone, Copy)]
pub enum PerformanceMode {
    /// No timing info (production default)
    Off,
    /// Show duration on span close only
    Compact,
    /// Show ENTER/EXIT events with timestamps
    Verbose,
}

pub struct LoggerConfig {
    whoami: String,
    otel_url: Option<String>,
    performance_mode: PerformanceMode,  // NEW
}

impl LoggerConfig {
    pub fn with_performance(mut self, mode: PerformanceMode) -> Self {
        self.performance_mode = mode;
        self
    }

    /// Preset: Production mode (no timing overhead)
    pub fn production() -> Self {
        Self {
            performance_mode: PerformanceMode::Off,
            // ...
        }
    }

    /// Preset: Development mode (show durations)
    pub fn development() -> Self {
        Self {
            performance_mode: PerformanceMode::Compact,
            // ...
        }
    }

    /// Preset: Performance debugging (verbose timing)
    pub fn performance_debug() -> Self {
        Self {
            performance_mode: PerformanceMode::Verbose,
            // ...
        }
    }
}

pub fn init(config: LoggerConfig) {
    let filt = tracing_subscriber::EnvFilter::from_default_env();

    // Configure span events based on performance mode
    let fmt_span = match config.performance_mode {
        PerformanceMode::Off => FmtSpan::NONE,
        PerformanceMode::Compact => FmtSpan::CLOSE,
        PerformanceMode::Verbose => FmtSpan::NEW | FmtSpan::CLOSE,
    };

    let stdout_sub = tracing_subscriber::fmt::layer()
        .compact()
        .with_span_events(fmt_span)  // NEW: Show span timing
        .with_filter(filt);

    // ... rest of initialization
}
```

**What this enables:**

**Mode: Off** (production default)
```
INFO  processed block
```
No timing overhead.

**Mode: Compact** (development)
```
INFO  process_block{component="asm_worker" l1_height=100}: close time.busy=234ms time.idle=12ms
INFO  processed block
```
Shows duration when span closes. Minimal overhead.

**Mode: Verbose** (performance debugging)
```
INFO  process_block{component="asm_worker" l1_height=100}: new
INFO  fetching block data
INFO  process_block{component="asm_worker" l1_height=100}: close time.busy=234ms time.idle=12ms
```
Shows span lifecycle with timestamps. Use for finding bottlenecks.

**Bonus: Add timing helper for non-span operations**

Add to `logging.rs`:
```rust
/// Helper for manually measuring operation duration
pub struct Timer {
    name: &'static str,
    start: std::time::Instant,
}

impl Timer {
    pub fn start(name: &'static str) -> Self {
        Self {
            name,
            start: std::time::Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        debug!(
            operation = self.name,
            duration_ms = elapsed.as_millis(),
            "operation completed"
        );
    }
}

// Usage:
// let _timer = Timer::start("expensive_operation");
// do_work();
// // Automatically logs duration on drop
```

**Integration with OpenTelemetry:**

The span durations are automatically captured by the OpenTelemetry subscriber (already configured lines 52-77) and sent to your tracing backend (Tempo/Jaeger) where you can:
- Build histograms of operation durations
- Track P50/P95/P99 latencies
- Set up alerts on slow operations
- Visualize trends over time

**Ticket:**
- **Title:** Add Automatic Performance Timing to Logger
- **Priority:** HIGH (critical for performance visibility)
- **Effort:** 4-5 hours
- **Files:** `crates/common/src/logging.rs`
- **Acceptance:**
  - `PerformanceMode` enum with Off/Compact/Verbose
  - `LoggerConfig::development()` shows span durations
  - `LoggerConfig::performance_debug()` shows ENTER/EXIT events
  - `Timer` helper for manual timing
  - Documentation showing output examples
  - Zero overhead when `PerformanceMode::Off`
  - Durations visible in logs: `time.busy=234ms`
  - Works with existing OpenTelemetry integration

#### B. JSON format support
```rust
// In Cargo.toml
tracing-subscriber = { workspace = true, features = ["json"] }

// In logging.rs
LogFormat::Json => fmt::layer()
    .json()
    .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
    .with_filter(filt)
    .boxed(),
```

**Why:** Machine-readable logs for Loki/Datadog/etc.

**Ticket:**
- **Title:** Add JSON Log Format Support
- **Priority:** Low
- **Effort:** 1-2 hours
- **Acceptance:** `RUST_LOG=debug cargo run` with JSON output works

---

### 2. Field Naming Conventions (Code + Documentation)

**Problem:**
Inconsistent names across codebase:
- Block IDs: `block`, `blkid`, `block_id`, `l1_block`, `blockid`
- Heights: `height`, `l1_height`, `block_height`, `block_number`
- Hashes: `hash`, `tx_hash`, `txhash`, `transaction_hash`

**Why documentation alone isn't enough:**
- Developers can still use wrong names (no compile-time checking)
- Typos slip through
- No IDE autocomplete
- Have to memorize or look up conventions

**Solution: String Constants Module**

Create `crates/common/src/fields.rs`:

```rust
//! Standard field names for structured logging and tracing.
//!
//! Use these constants instead of raw strings to ensure consistency
//! and get compile-time checking + IDE autocomplete.

/// Component identification
pub const COMPONENT: &str = "component";
pub const OPERATION: &str = "operation";

/// L1 (Bitcoin) identifiers - NEVER abbreviated
pub const L1_BLOCK: &str = "l1_block";
pub const L1_HEIGHT: &str = "l1_height";
pub const L1_TX_HASH: &str = "l1_tx_hash";

/// L2 (Strata) identifiers - NEVER abbreviated
pub const L2_BLOCK: &str = "l2_block";
pub const L2_SLOT: &str = "l2_slot";
pub const L2_TX_HASH: &str = "l2_tx_hash";

/// Epoch and checkpoint
pub const EPOCH: &str = "epoch";
pub const CHECKPOINT_IDX: &str = "checkpoint_idx";

/// Status and metrics
pub const STATUS: &str = "status";
pub const DURATION_MS: &str = "duration_ms";
pub const ERROR: &str = "error";
pub const ERROR_KIND: &str = "error_kind";

/// RPC and networking
pub const RPC_METHOD: &str = "rpc_method";
pub const PEER_ID: &str = "peer_id";
pub const RETRY_COUNT: &str = "retry_count";
```

**Usage:**

```rust
use strata_common::fields;

// Before (error-prone):
info!(component = "asm_worker", l1_block = %block_id, "processed");

// After (compile-time checked):
info!({fields::COMPONENT} = "asm_worker", {fields::L1_BLOCK} = %block_id, "processed");

// Or with spans:
let span = info_span!(
    "process_block",
    {fields::COMPONENT} = "asm_worker",
    {fields::L1_BLOCK} = %block_id,
    {fields::L1_HEIGHT} = height,
);
```

**Why string constants instead of Enum?**

❌ **Enum approach** (over-engineered):
```rust
pub enum Field {
    Component,
    L1Block,
    // ...
}

impl Field {
    pub fn as_str(&self) -> &'static str { /* ... */ }
}

// Usage is verbose:
info!({Field::L1Block.as_str()} = %id, "log");
```

Problems:
- Verbose (need `.as_str()` everywhere)
- Doesn't work cleanly with tracing field syntax
- Over-engineered for simple string constants

✅ **String constants** (right level of abstraction):
- Works directly with tracing macros
- IDE autocomplete: type `fields::` and see all options
- Compile-time errors if you typo the constant name
- Zero runtime overhead (constants inlined)
- Simple to maintain

**Ticket:**
- **Title:** Add Field Name Constants Module
- **Priority:** High (enables consistency)
- **Effort:** 2-3 hours
- **Files:**
  - Create `crates/common/src/fields.rs`
  - Update `crates/common/src/lib.rs`: add `pub mod fields;`
  - Create `FIELD-CONVENTIONS.md` documenting the constants
- **Acceptance:**
  - `strata_common::fields` module exists with all standard field names
  - Documentation explains when to use which field
  - Example showing usage in spans and events
  - Can import and use: `use strata_common::fields;`

---

### 3. Component Naming Standards (Documentation)

**Problem:**
No standard for which component name to use where.

**Solution:**
Create `COMPONENT-NAMES.md`:

| Code Location | Component Name | Purpose |
|--------------|----------------|---------|
| `crates/asm/worker/` | `asm_worker` | L1 anchor state machine |
| `crates/csm-worker/` | `csm_worker` | Checkpoint state machine |
| `crates/btcio/reader/` | `l1_reader` | Bitcoin block reader |
| `crates/consensus-logic/fork_choice*` | `fork_choice` | Fork choice (all 3 modules) |
| `crates/consensus-logic/chain_worker*` | `chain_worker` | Chain worker |
| `crates/consensus-logic/sync_manager*` | `sync_manager` | Sync manager |
| `crates/sequencer/block_template/` | `block_template` | Block template generation |
| `crates/sequencer/checkpoint/` | `checkpoint_worker` | Checkpoint worker |
| `crates/eectl/` | `evm_engine` | EVM execution engine |
| `crates/chaintsn/` | `state_transition` | State transition logic |
| `crates/storage/` | `storage` | Storage layer |
| RPC client | `rpc_client` | RPC client |
| RPC server | `rpc_server` | RPC server |

**Rule:** Multiple modules implementing ONE logical component use the SAME component name.

**Ticket:**
- **Title:** Document Component Naming Standards
- **Priority:** High (unblocks team)
- **Effort:** 1-2 hours
- **Deliverable:** `COMPONENT-NAMES.md` document
- **Acceptance:** Team knows which component name to use

---

## Critical Fixes (Do First)

## Actionable Tickets (Prioritized)

### Tier 1: Foundation (Week 1) - Unblocks Everything

#### Ticket 1.0: Adopt Mandatory Observability Guidelines
**Problem:** No enforceable standards - developers don't know what's required
**Priority:** CRITICAL (unblocks all other tickets)
**Effort:** 1 hour (team meeting + PR process update)

**What to do:**

1. **Team Review:** Hold a 30-minute meeting to review `guidelines.md`
   - Walk through each mandatory requirement
   - Answer questions
   - Get team buy-in

2. **Update PR Template:** Add observability checklist to PR template:
```markdown
## Observability Checklist (See guidelines.md)
- [ ] Significant functions have spans
- [ ] All spans have `{fields::COMPONENT}`
- [ ] Natural IDs used for correlation (l1_block, tx_hash, etc.)
- [ ] No abbreviated IDs in logs
- [ ] Using `strata_common::fields` constants
- [ ] Hot loops have rate limiting
- [ ] Errors include context
- [ ] Cross-component calls propagate correlation
```

3. **Update CONTRIBUTING.md:** Link to `guidelines.md` as mandatory reading

4. **Communicate:** Announce on team channel that these are now MANDATORY for all new code

**Acceptance:**
- Team has reviewed `guidelines.md`
- PR template includes observability checklist
- CONTRIBUTING.md links to guidelines
- New PRs are being checked against guidelines

**Why this comes first:** Without adopted standards, the other tickets will just recreate the same problems.

---

#### Ticket 1.1: Fix Abbreviated Block IDs
**Problem:** `blkid=aa026e..91422d` breaks grep
**Priority:** CRITICAL
**Effort:** 2-3 hours
**Files to change:**
```bash
# Find all abbreviated IDs
$ grep -r '\.\.\"' crates/ | grep -E "(block|blkid|hash|txid)"
```

**What to do:**
- Replace all abbreviated block IDs with full IDs
- Use consistent field names: `l1_block`, `l2_block`, `tx_hash`
- Never use `..` in log output

**Acceptance:**
- Can grep any full block ID and find all logs
- No `..` in any block/tx ID logs

---

#### Ticket 1.2: Add Field Name Constants Module
**Problem:** Inconsistent field names (`block`, `blkid`, `block_id`, `l1_block`) - documentation alone doesn't prevent typos
**Priority:** HIGH
**Effort:** 2-3 hours

**What to do:**

1. Create `crates/common/src/fields.rs` with string constants:
```rust
pub const COMPONENT: &str = "component";
pub const L1_BLOCK: &str = "l1_block";
pub const L1_HEIGHT: &str = "l1_height";
pub const L2_BLOCK: &str = "l2_block";
pub const L2_SLOT: &str = "l2_slot";
pub const EPOCH: &str = "epoch";
pub const CHECKPOINT_IDX: &str = "checkpoint_idx";
pub const STATUS: &str = "status";
pub const DURATION_MS: &str = "duration_ms";
pub const ERROR: &str = "error";
pub const RPC_METHOD: &str = "rpc_method";
// ... (see section 2 for full list)
```

2. Update `crates/common/src/lib.rs`:
```rust
pub mod fields;
```

3. Create `FIELD-CONVENTIONS.md` documenting each constant and when to use it.

**Benefits:**
- IDE autocomplete: type `fields::` and see all options
- Compile-time errors on typos
- No need to memorize field names
- Enforced consistency across codebase

**Acceptance:**
- Can import: `use strata_common::fields;`
- All standard field names defined as constants
- Documentation explains usage
- Example code showing spans with field constants

---

#### Ticket 1.3: Document Component Naming Standards
**Problem:** No standard for component names
**Priority:** HIGH
**Effort:** 1 hour
**Deliverable:** `COMPONENT-NAMES.md`

**Content:** See table in section 3 above

**Acceptance:**
- Document exists
- Team knows which component to use for each crate

---

#### Ticket 1.4: Add Automatic Performance Timing to Logger
**Problem:** No visibility into operation duration, bottlenecks, or performance trends
**Priority:** HIGH (critical for performance visibility)
**Effort:** 4-5 hours
**Files:** `crates/common/src/logging.rs`

**What to do:**

1. Add `PerformanceMode` enum to `logging.rs`:
```rust
#[derive(Debug, Clone, Copy)]
pub enum PerformanceMode {
    Off,      // Production: no timing overhead
    Compact,  // Dev: show duration on span close
    Verbose,  // Debug: show ENTER/EXIT with timestamps
}
```

2. Update `LoggerConfig`:
```rust
pub struct LoggerConfig {
    whoami: String,
    otel_url: Option<String>,
    performance_mode: PerformanceMode,  // NEW field
}

impl LoggerConfig {
    pub fn production() -> Self { /* PerformanceMode::Off */ }
    pub fn development() -> Self { /* PerformanceMode::Compact */ }
    pub fn performance_debug() -> Self { /* PerformanceMode::Verbose */ }
}
```

3. Modify `init()` function (line 43) to enable span timing:
```rust
use tracing_subscriber::fmt::format::FmtSpan;

pub fn init(config: LoggerConfig) {
    let fmt_span = match config.performance_mode {
        PerformanceMode::Off => FmtSpan::NONE,
        PerformanceMode::Compact => FmtSpan::CLOSE,
        PerformanceMode::Verbose => FmtSpan::NEW | FmtSpan::CLOSE,
    };

    let stdout_sub = tracing_subscriber::fmt::layer()
        .compact()
        .with_span_events(fmt_span)  // ADD THIS
        .with_filter(filt);
    // ...
}
```

4. (Optional) Add `Timer` helper struct for manual timing

**What this enables:**

```rust
// In application code:
let span = info_span!("process_block", component = "asm_worker", l1_height = 100);
async move {
    info!("processing started");
    do_work().await?;
    info!("processing done");
}
.instrument(span)
.await
```

**Output with PerformanceMode::Compact:**
```
INFO  process_block{component="asm_worker" l1_height=100}: close time.busy=234ms time.idle=12ms
INFO  processing done
```

**Benefits:**
- See **how long** each operation took (`time.busy=234ms`)
- See **where time was spent** (which spans are slow)
- Track **trends over time** via OpenTelemetry histograms
- Zero overhead in production mode
- Automatic - no manual timing code needed

**Acceptance:**
- `PerformanceMode` enum exists with three modes
- `LoggerConfig::development()` shows span close timing
- `LoggerConfig::performance_debug()` shows span enter/exit
- Logs display `time.busy` and `time.idle` fields
- Zero overhead when `PerformanceMode::Off`
- Works with existing OpenTelemetry integration
- Documentation showing all three modes with output examples

**See:** Section 1.A "Performance Visibility - Automatic Span Timing" for detailed design.

---

### Tier 2: Core Instrumentation (Week 2-3) - Prove the Pattern

####Ticket 2.1: Instrument L1 Pipeline with Spans
**Problem:** ASM → CSM logs are disconnected, manual field duplication
**Priority:** HIGH
**Effort:** 6-8 hours

**Files to change:**
1. `crates/asm/worker/src/service.rs:37-104` - `process_input()`
2. `crates/csm-worker/src/service.rs:34-71` - `process_input()`
3. Add `req_id` field to `AsmWorkerStatus` (or use block ID)

**Before:**
```rust
// crates/asm/worker/src/service.rs
fn process_input(state: &mut State, block: &L1BlockCommitment) -> Result<Response> {
    info!(%height, "ignoring unexpected L1 block");  // No component, no correlation
    // ...
}
```

**After:**
```rust
fn process_input(state: &mut State, block: &L1BlockCommitment) -> Result<Response> {
    let span = info_span!(
        "process_l1_block",
        component = "asm_worker",
        l1_block = %block.block_id,  // Natural ID for correlation
        l1_height = block.height,
    );

    async move {
        info!("ASM processing L1 block");
        // All logs here inherit component, l1_block, l1_height

        let anchor = find_pivot_anchor(state, block).await?;
        info!(pivot_block = %anchor.block_id, "found pivot anchor");

        state.transition(block)?;
        info!("ASM transition complete");

        Ok(Response::Continue)
    }
    .instrument(span)
    .await
}
```

**Acceptance Criteria:**
- [ ] One span per component (ASM, CSM)
- [ ] `component` field on all spans
- [ ] Natural identifier (`l1_block`) on all spans
- [ ] All logs inherit span fields (no manual duplication)
- [ ] Can grep `l1_block=...` to see ASM → CSM flow
- [ ] Can grep `component=asm_worker` to see all ASM activity

**Success metric:**
```bash
$ grep "l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d" logs
INFO component=asm_worker l1_block=aa026ef... l1_height=100: ASM processing L1 block
INFO component=asm_worker l1_block=aa026ef... l1_height=100: found pivot anchor
INFO component=csm_worker l1_block=aa026ef... l1_height=100: CSM processing logs
INFO component=csm_worker l1_block=aa026ef... l1_height=100: found checkpoint epoch=42
```

---

#### Ticket 2.2: Instrument Fork Choice Manager
**Problem:** Fork choice spans 3 modules with different module paths
**Priority:** MEDIUM
**Effort:** 4-5 hours

**Files to change:**
1. `crates/consensus-logic/src/fork_choice_manager.rs`
2. `crates/consensus-logic/src/unfinalized_tracker.rs`
3. `crates/consensus-logic/src/tip_update.rs`

**Pattern:**
```rust
// All 3 files use SAME component name
let span = info_span!(
    "operation_name",
    component = "fork_choice",  // Same across all modules
    l1_height = height,
    // Add operation-specific fields
);
```

**Acceptance:**
- [ ] All 3 modules use `component = "fork_choice"`
- [ ] Can grep `component=fork_choice` to see all fork choice activity
- [ ] Reorg detection includes `old_tip`, `new_tip`, `reorg_depth`

---

#### Ticket 2.3: Add Rate Limiting to Hot Loops
**Problem:** Loops log every iteration, creating noise
**Priority:** MEDIUM
**Effort:** 3-4 hours

**Find hot loops:**
```bash
$ grep -r "for.*in.*\(0\.\|range\|iter\)" crates/ | grep -B3 -A3 "info!\|debug!"
```

**Pattern:**
```rust
// BEFORE: 1000 logs
for i in 0..1000 {
    debug!("processing {}", i);  // Too much!
}

// AFTER: ~100 logs
let mut processed = 0;
for item in items {
    process(item)?;
    processed += 1;

    if processed % 10 == 0 {  // Log every 10th
        debug!(processed, "batch progress");
    }
}
info!(total = processed, "batch complete");
```

**Acceptance:**
- [ ] Hot loops identified
- [ ] Rate limiting added (log every Nth iteration)
- [ ] Final summary always logged
- [ ] 10x log reduction in batch processing

---

### Tier 3: Advanced Features (Week 4-5)

#### Ticket 3.1: Instrument Block Template Generation
**Problem:** Block assembly crosses 4 crates, appears disconnected
**Priority:** MEDIUM
**Effort:** 6-8 hours

**Files:**
1. `crates/sequencer/src/block_template/block_assembly.rs:74-162` - Main `prepare_block()`
2. `crates/consensus-logic/src/checkpoint_verification.rs:20-41` - Checkpoint verification
3. Integration with EECtl, ChainTsn

**Pattern:**
```rust
pub fn prepare_block(...) -> Result<(L2BlockHeader, L2BlockBody, L2BlockAccessory)> {
    let span = info_span!(
        "generate_block_template",
        component = "block_template",
        l2_slot = slot,
        prev_block = %prev_blkid,
    );

    async move {
        info!("preparing L2 block");

        // Stage 1: L1 segment
        let l1_seg = prepare_l1_segment(...).instrument(info_span!(
            "prepare_l1_segment",
            component = "block_template",
        )).await?;

        // Stage 2: EVM execution
        let exec_seg = prepare_exec_data(...).instrument(info_span!(
            "execute_evm",
            component = "block_template",
        )).await?;

        // Stage 3: State transition
        let post_state = compute_post_state(...).instrument(info_span!(
            "compute_state",
            component = "block_template",
        )).await?;

        info!(state_root = %post_state.root, "block prepared");
        Ok((header, body, accessory))
    }
    .instrument(span)
    .await
}
```

**Acceptance:**
- [ ] One root span with `component = "block_template"`
- [ ] Child spans for each stage
- [ ] Automatic timing for each stage
- [ ] Can trace one template generation: `grep "l2_slot=50"`

---

#### Ticket 3.2: Add Verbose Logging Mode
**Problem:** Developers need verbose mode for debugging
**Priority:** MEDIUM
**Effort:** 3-4 hours

**Files:**
1. `crates/common/src/logging.rs` - Add configuration
2. Binary `main.rs` files - Add `--verbose` flag

**Implementation:** See section 1.A above (Verbose logging configuration)

**Acceptance:**
- [ ] `--verbose` flag works in binaries
- [ ] Shows span creation/closure events
- [ ] Shows line numbers
- [ ] Pretty colored output
- [ ] Default mode unchanged

---

#### Ticket 3.3: Add JSON Log Format Support
**Problem:** Need machine-readable logs for Loki/Datadog
**Priority:** LOW
**Effort:** 1-2 hours

**Implementation:** See section 1.B above

**Acceptance:**
- [ ] JSON format available
- [ ] All fields properly structured
- [ ] Can parse with `jq`

---

#### Ticket 3.4: Add RPC Distributed Tracing
**Problem:** RPC calls lose trace context across service boundaries
**Priority:** MEDIUM
**Effort:** 8-10 hours

**Approach:** OpenTelemetry already configured in `logging.rs`

**What to do:**
1. Ensure HTTP client propagates trace context (most clients do automatically)
2. Ensure HTTP server extracts trace context (most servers do automatically)
3. Test that `trace_id` flows across RPC boundaries

**If using custom RPC:**
- Client: Inject W3C `traceparent` header
- Server: Extract `traceparent` header, create span with extracted context

**Acceptance:**
- [ ] Same `trace_id` appears in client and server logs
- [ ] Can trace request across network: `grep "trace_id=..."`
- [ ] Works for key RPC methods

---

### Tier 4: Ecosystem Integration (Week 6+)

#### Ticket 4.1: Set Up Grafana + Loki
**Priority:** LOW
**Effort:** 4-6 hours (ops work)

**Steps:**
1. Deploy Loki for log aggregation
2. Deploy Grafana for visualization
3. Configure log shipping (already JSON-capable)
4. Create dashboards:
   - Component activity over time
   - Error rates by component
   - Operation latency (from span duration)
   - L1/L2 block processing rates

**Acceptance:**
- [ ] Logs flowing to Loki
- [ ] Can query by `{component="asm_worker"}`
- [ ] Dashboards show key metrics

---

#### Ticket 4.2: Add Custom Subscribers for Business Logic
**Priority:** LOW
**Effort:** Varies by use case

**Examples:**
- Alert on checkpoint timeout
- Metrics extraction (histogram of block processing times)
- Custom tracing for debugging specific issues

**Pattern:**
```rust
struct CustomSubscriber;

impl<S: Subscriber> Layer<S> for CustomSubscriber {
    fn on_event(&self, event: &Event, ctx: Context<S>) {
        // Custom logic here
        // E.g., send alert if error in checkpoint_worker
    }
}

// In logging.rs init()
let custom = CustomSubscriber::new();
registry
    .with(fmt_layer)
    .with(otel_layer)
    .with(custom)  // Add custom subscriber
    .init();
```

---

## Correlation Strategies

### Strategy 1: Natural Identifiers (Recommended)
Use entity IDs that already exist:
- L1 block processing → `l1_block` (the block ID itself)
- Checkpoint processing → `epoch` or `checkpoint_idx`
- Fork choice → `l1_height`

**Pros:**
- Meaningful (block ID has business meaning)
- No generation needed
- Already unique

**Cons:**
- Some operations might not have natural ID

**When to use:** Default approach for entity-based operations

---

### Strategy 2: Generate Request IDs
For operations without natural IDs, generate a correlation ID:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_req_id() -> String {
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    format!("{:08x}{:08x}", ts as u32, count as u32)
}
```

**When to use:**
- RPC calls (no natural ID)
- Background tasks
- Operations spanning multiple entities

---

### Strategy 3: OpenTelemetry Trace IDs
For distributed tracing across services, use OpenTelemetry's trace IDs:

**Already configured** in `logging.rs`:
```rust
let otel_sub = tracing_opentelemetry::layer().with_tracer(tt);
```

OpenTelemetry automatically:
- Generates W3C trace IDs
- Propagates them in HTTP headers
- Links parent/child spans

**When to use:** Cross-service RPC calls

---

## Implementation Order (Recommended)

### Sprint 1 (Foundation):
1. Ticket 1.1: Fix abbreviated IDs ← **Critical**
2. Ticket 1.2: Document field conventions
3. Ticket 1.3: Document component names

**Deliverables:** Documentation, immediate grep fixes

---

### Sprint 2 (Core Pattern):
4. Ticket 2.1: Instrument L1 pipeline ← **Proves pattern**
5. Ticket 2.3: Add rate limiting

**Deliverables:** One complete instrumented flow (ASM → CSM)

---

### Sprint 3 (Expand Coverage):
6. Ticket 2.2: Instrument fork choice
7. Ticket 3.1: Instrument block template
8. Ticket 3.2: Add verbose mode

**Deliverables:** Multiple instrumented flows, better DX

---

### Sprint 4 (Advanced):
9. Ticket 3.3: JSON format
10. Ticket 3.4: RPC distributed tracing
11. Ticket 4.1: Grafana + Loki setup

**Deliverables:** Production-ready observability

---

## Success Metrics

### Before:
```
INFO strata_asm_worker::service: processing block
INFO strata_csm_worker::service: CSM processing logs
```
- ❌ Can't correlate ASM → CSM
- ❌ No component tags
- ❌ Can't grep block IDs (abbreviated)

### After:
```
INFO component=asm_worker l1_block=aa026ef... l1_height=100: processing L1 block
INFO component=csm_worker l1_block=aa026ef... l1_height=100: CSM processing logs
INFO component=csm_worker l1_block=aa026ef... l1_height=100 epoch=42: found checkpoint
```
- ✅ Can correlate: `grep "l1_block=aa026ef..."`
- ✅ Can filter: `grep "component=asm_worker"`
- ✅ Can trace: `grep "epoch=42"`

### Quantitative:
- **Time to root cause:** Hours → Minutes
- **Cross-crate visibility:** 0% → 100%
- **Log noise:** High → Low (10x reduction with rate limiting)
- **Grep success rate:** ~30% → ~95%

---

## Key Principles

1. **Start Simple** - Use tracing directly, no frameworks yet
2. **Component Tags** - Semantic grouping, not module paths
3. **Spans Over Manual Fields** - Set context once, inherit everywhere
4. **Full IDs** - Storage is cheap, debugging time is expensive
5. **Natural Identifiers** - Use entity IDs when available
6. **Rate Limit Hot Loops** - Summary + periodic updates
7. **Document Standards** - Field names, component names
8. **Prove Pattern First** - Instrument one flow end-to-end, then expand

---

## Questions to Answer Before Starting

### Q: Do we need helper infrastructure in strata-common?
**A:** Not yet. Start with raw tracing. Add helpers if you see duplication.

Options if needed later:
- **Option A:** Constants (`pub const ASM_WORKER: &str = "asm_worker"`)
- **Option B:** Enum (`enum Component { AsmWorker, ... }`)
- **Option C:** Full framework (only if A/B insufficient)

### Q: Should we use natural IDs or generate request IDs?
**A:** Prefer natural IDs (block ID, epoch). Generate IDs only for operations without natural entities (RPC calls, background tasks).

### Q: Do we need to instrument everything?
**A:** No. Start with critical paths (L1 pipeline, fork choice, block template). Expand based on debugging needs.

### Q: How much will this impact performance?
**A:** Minimal:
- Span creation: ~100 bytes, microsecond overhead
- Structured logging: 1-3% CPU
- OpenTelemetry: negligible when sampling

Trade-off: 1-3% always-on cost vs hours/days debugging without observability.

### Q: Should we do this incrementally or all at once?
**A:** Incremental. Instrument one flow end-to-end (L1 pipeline), prove it works, then expand.

---

## Anti-Patterns to Avoid

### ❌ Random UUID Generation for Everything
Don't generate UUIDs when you have natural identifiers.

### ❌ Manual Field Duplication
Don't repeat fields on every log. Use spans.

### ❌ Abbreviated IDs
Never abbreviate block/tx IDs. Breaks grep.

### ❌ Magic Strings
Use constants or enums for component names.

### ❌ Logging Every Iteration
Use rate limiting in hot loops.

### ❌ Big Bang Approach
Don't try to instrument everything at once. One flow at a time.

---

## Resources

- **Tracing crate docs:** https://docs.rs/tracing
- **OpenTelemetry:** Already configured in `logging.rs`
- **W3C Trace Context:** https://www.w3.org/TR/trace-context/
- **Internal docs:**
  - `improvements.md` - Code examples
  - `improvements-in-words.md` - Problem explanations
  - `standards-to-follow.md` - Detailed standards
  - `TRACING-QUICKSTART.md` - Developer quick start

---

## Summary

**Infrastructure needed:**
1. Extend `logging.rs` with verbose/JSON modes (optional, nice-to-have)
2. Document field naming conventions (critical)
3. Document component names (critical)

**Critical work:**
1. Fix abbreviated IDs (immediate)
2. Instrument L1 pipeline (proves pattern)
3. Add component tags everywhere
4. Convert to span-based instrumentation

**Advanced:**
1. RPC distributed tracing
2. Grafana + Loki
3. Custom subscribers

**Timeline:** 4-6 weeks for core observability, 8-10 weeks for full stack.

**Start with:** Fix abbreviated IDs + document conventions (Week 1).
