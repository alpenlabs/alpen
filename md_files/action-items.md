# Observability Action Items - Implementation Roadmap

**Status:** Implementation Plan
**Last Updated:** 2025-12-10

## Overview

Incremental approach to observability that acknowledges ongoing refactors. Each phase builds foundation for the next, with concrete references and verification steps.

**Team Consensus:**
- **eivgeniy:** Cross-service spans for critical paths, basic metrics at service framework level
- **trey:** Leverage tracing metrics system with Status concept
- **sam:** Security-critical metrics for incident response, anomaly detection, audit capability

---

## Phase 1: OpenTelemetry Infrastructure Foundation

**Objective:** Properly configure the tracing-subscriber layer architecture and OTLP pipeline to support structured tracing.

**Why First:** Everything else depends on this. Without proper subscriber setup, spans won't be recorded, context won't propagate, and metrics won't export.

**Effort:** 1 day (includes reading, understanding, testing)

### Background Reading (Required)

#### 1.1 Tracing Subscriber Architecture

**Core Concept:** `tracing-subscriber` uses a **layer-based architecture** where multiple layers can process the same tracing data.

**Read:**
- **Tracing Subscriber Docs**: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/
  - Section: "Composing layers" - explains `Layer` trait and composition
  - Section: "Using the registry" - explains `Registry` as the base subscriber
- **Tracing Subscriber Layer Trait**: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/layer/trait.Layer.html
  - Shows how layers intercept span creation, entry, exit, events

**Key Architecture:**
```
Registry (base subscriber)
  ├─ fmt::Layer (stdout logging)
  ├─ EnvFilter (filtering logic)
  └─ OpenTelemetryLayer (OTLP export)
```

Each layer sees the same span/event but processes it differently.

**Current State** (`crates/common/src/logging.rs:42-83`):
```rust
// We have:
let stdout_sub = tracing_subscriber::fmt::layer().compact().with_filter(filt);
let otel_sub = tracing_opentelemetry::layer().with_tracer(tt);

tracing_subscriber::registry()
    .with(stdout_sub)
    .with(otel_sub)
    .init();
```

**What's Missing:**
- No `FmtSpan` configuration - spans don't log their duration
- No trace context propagator - W3C trace context won't flow across processes
- No resource attributes beyond service.name
- No proper shutdown of OTLP exporter

#### 1.2 OpenTelemetry Trace Context Propagation

**Core Concept:** W3C Trace Context defines HTTP headers (`traceparent`, `tracestate`) that carry trace IDs across service boundaries.

**Read:**
- **W3C Trace Context Spec**: https://www.w3.org/TR/trace-context/
  - Section 2: "traceparent Header" - format: `00-{trace-id}-{parent-id}-{flags}`
  - Section 3: "tracestate Header" - vendor-specific baggage
- **OpenTelemetry Context Propagation**: https://opentelemetry.io/docs/specs/otel/context/api-propagators/
  - Explains inject/extract pattern for propagation
- **Rust Implementation**: https://docs.rs/opentelemetry/latest/opentelemetry/propagation/trait.TextMapPropagator.html

**How It Works:**
1. Service A creates a span → generates trace ID
2. Service A **injects** trace context into outbound request headers
3. Service B **extracts** trace context from inbound request headers
4. Service B's spans become children of Service A's span

**Code Pattern:**
```rust
use opentelemetry::global;
use opentelemetry::propagation::TextMapPropagator;

// At startup: set global propagator
opentelemetry::global::set_text_map_propagator(
    opentelemetry_sdk::propagation::TraceContextPropagator::new()
);

// In client: inject context into HTTP headers
let propagator = global::get_text_map_propagator(|p| p.clone());
propagator.inject_context(&current_context, &mut headers);

// In server: extract context from HTTP headers
let propagator = global::get_text_map_propagator(|p| p.clone());
let parent_ctx = propagator.extract(&headers);
```

**Reference:** https://docs.rs/opentelemetry-sdk/latest/opentelemetry_sdk/propagation/struct.TraceContextPropagator.html

#### 1.3 Span Events and Duration Logging

**Core Concept:** `FmtSpan` controls when span lifecycle events are logged (NEW, ENTER, EXIT, CLOSE) and whether duration is included.

**Read:**
- **FmtSpan Docs**: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/format/struct.FmtSpan.html
- **Tracing Span Lifecycle**: https://docs.rs/tracing/latest/tracing/span/index.html#span-lifecycle

**Span Lifecycle:**
```
NEW    → span created
ENTER  → entered (.enter() called or .instrument())
EXIT   → exited (guard dropped)
CLOSE  → all references dropped, duration calculated
```

**Configuration Options:**
```rust
use tracing_subscriber::fmt::format::FmtSpan;

// Log when span is entered
FmtSpan::ENTER

// Log when span exits (includes duration since enter)
FmtSpan::EXIT

// Log when span closes (includes total duration)
FmtSpan::CLOSE

// Combine multiple
FmtSpan::NEW | FmtSpan::CLOSE
```

**Why CLOSE is useful:** Shows total duration even if span is entered/exited multiple times.

**Reference:** https://tokio.rs/blog/2019-08-tracing#visualizing-async-programs

#### 1.4 OpenTelemetry Resource Attributes

**Core Concept:** Resource attributes are metadata about your service (name, version, environment) sent with every span.

**Read:**
- **OpenTelemetry Resource Spec**: https://opentelemetry.io/docs/specs/otel/resource/semantic_conventions/
  - Section: "Service" - service.name, service.version, service.instance.id
  - Section: "Deployment" - deployment.environment
- **Rust SDK Resource**: https://docs.rs/opentelemetry-sdk/latest/opentelemetry_sdk/struct.Resource.html

**Semantic Conventions:**
```
service.name              = "strata-client"
service.version           = "0.1.0"
service.instance.id       = "strata-client-1"
deployment.environment    = "dev" | "staging" | "production"
```

These show up in Grafana/Jaeger as filterable fields.

**Reference:** https://opentelemetry.io/docs/specs/semconv/resource/

### Tasks

#### 1.1 Enhanced Logging Configuration

**File:** `crates/common/src/logging.rs`

**Changes Needed:**
1. Add `TraceContextPropagator` as global propagator
2. Add `FmtSpan::CLOSE` to log span durations
3. Add comprehensive resource attributes
4. Add JSON output mode for production
5. Implement proper shutdown

**Implementation:**

```rust
// Add imports
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing_subscriber::fmt::format::FmtSpan;

// Add field to LoggerConfig
#[derive(Debug)]
pub struct LoggerConfig {
    whoami: String,
    otel_url: Option<String>,
    pub json_output: bool,  // NEW
}

impl LoggerConfig {
    pub fn set_json_output(&mut self, enabled: bool) {
        self.json_output = enabled;
    }
}

pub fn init(config: LoggerConfig) {
    // 1. Set global propagator for W3C trace context
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    let filt = tracing_subscriber::EnvFilter::from_default_env();

    // 2. Enhanced stdout layer with span events
    let stdout_sub = if config.json_output {
        tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)        // Include current span info
            .with_span_list(true)           // Include full span hierarchy
            .with_span_events(FmtSpan::CLOSE)  // Log span duration on close
            .with_filter(filt.clone())
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .compact()
            .with_span_events(FmtSpan::CLOSE)  // Log span duration on close
            .with_filter(filt)
            .boxed()
    };

    if let Some(otel_url) = &config.otel_url {
        // 3. Enhanced resource attributes
        let trace_config = opentelemetry_sdk::trace::Config::default()
            .with_resource(opentelemetry_sdk::Resource::new(vec![
                opentelemetry::KeyValue::new("service.name", config.whoami.clone()),
                opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                opentelemetry::KeyValue::new(
                    "deployment.environment",
                    std::env::var("DEPLOYMENT_ENV").unwrap_or_else(|_| "dev".into())
                ),
                opentelemetry::KeyValue::new(
                    "service.instance.id",
                    format!("{}-{}", config.whoami, std::process::id())
                ),
            ]))
            .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn);

        let exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(otel_url);

        let tp = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(exporter)
            .with_trace_config(trace_config)
            .install_batch(opentelemetry_sdk::runtime::Tokio)
            .expect("failed to initialize opentelemetry pipeline");

        let tracer = tp.tracer("strata-log");

        let otel_sub = tracing_opentelemetry::layer()
            .with_tracer(tracer)
            .with_tracked_inactivity(true);  // Track inactive spans

        tracing_subscriber::registry()
            .with(stdout_sub)
            .with(otel_sub)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(stdout_sub)
            .init();
    }

    info!(whoami = %config.whoami, "logging initialized with enhanced tracing");
}

// 4. Proper shutdown
pub fn finalize() {
    info!("shutting down logging");
    // Flush and shutdown OpenTelemetry
    opentelemetry::global::shutdown_tracer_provider();
}
```

**References for Verification:**
- TraceContextPropagator: https://docs.rs/opentelemetry-sdk/0.21.0/opentelemetry_sdk/propagation/struct.TraceContextPropagator.html
- FmtSpan: https://docs.rs/tracing-subscriber/0.3.18/tracing_subscriber/fmt/format/struct.FmtSpan.html
- Resource: https://docs.rs/opentelemetry-sdk/0.21.0/opentelemetry_sdk/struct.Resource.html
- Shutdown: https://docs.rs/opentelemetry/1.0.0/opentelemetry/global/fn.shutdown_tracer_provider.html

#### 1.2 Verification Steps

**Test 1: Span Duration Logging**
```bash
# Run any service with a span
RUST_LOG=info cargo run --bin strata-client

# You should see:
# INFO some_operation{component="asm_worker"}: close time.busy=123ms time.idle=45ms
```

**Test 2: OpenTelemetry Resource Attributes**

If you have Grafana/Jaeger set up:
```bash
# Query traces filtered by resource attributes
# In Grafana Tempo: {resource.service.name="strata-client"}
```

**Test 3: Trace Context Propagation**
```rust
// Create a test span and verify traceparent header is generated
use opentelemetry::propagation::TextMapPropagator;
use std::collections::HashMap;

let span = info_span!("test");
let _guard = span.enter();

let propagator = opentelemetry::global::get_text_map_propagator(|p| p.clone());
let mut carrier = HashMap::new();
propagator.inject_context(&opentelemetry::Context::current(), &mut carrier);

println!("traceparent: {:?}", carrier.get("traceparent"));
// Should print: traceparent: Some("00-<trace-id>-<span-id>-01")
```

**Expected Output Format:**
```
traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
             ││  └─ trace id (32 hex chars)         │                │
             ││                                      └─ parent span id│
             ││                                                       └─ trace flags
             │└─ version
             └─ format
```

**Test 4: JSON Output Mode**
```bash
# Run with JSON output
RUST_LOG=info cargo run --bin strata-client -- --json-logs

# Output should be JSON:
# {"timestamp":"2025-12-10T...","level":"INFO","fields":{"component":"asm_worker"},"target":"strata_asm_worker","span":{"name":"process_input"},"spans":[...]}
```

---

## Phase 2: Service Framework Auto-Instrumentation

**Objective:** Instrument the service framework itself so all services inheriting from it get spans automatically.

**Why Second:** This is the highest leverage change. Once done, ASM worker, CSM worker, etc. all get basic instrumentation without per-service code changes.

**Dependency:** Phase 1 must be complete (otherwise spans won't have duration or proper context).

**Effort:** 2 days (includes coordination with strata-common repo)

### Background Reading (Required)

#### 2.1 The Service Framework Architecture

**Current State:**
- `strata-service` crate lives in external repo: `alpenlabs/strata-common`
- Defines `Service` and `SyncService` traits
- All workers (ASM, CSM, etc.) implement these traits
- Framework handles message dispatch, but doesn't instrument it

**Read:**
- **Trait-based instrumentation pattern**: https://docs.rs/tracing/latest/tracing/attr.instrument.html#instrumenting-trait-implementations
- **Async trait instrumentation**: https://github.com/tokio-rs/tracing/issues/2047#issuecomment-1085836950

**Challenge:** The service framework is generic over `Service` trait. We need to instrument the generic message handler without requiring each service to manually add spans.

**Pattern:**
```rust
// Framework provides instrumented wrapper
pub fn handle_message<S: SyncService>(
    state: &mut S::State,
    msg: &S::Msg,
) -> anyhow::Result<Response> {
    // Framework creates span automatically
    let span = info_span!(
        "service_message",
        component = S::component_name(),  // Each service must provide this
        otel.kind = "internal",
    );

    let _guard = span.enter();
    S::process_input(state, msg)  // Call user implementation
}
```

#### 2.2 Instrumentation Macro Patterns

**Read:**
- **#[instrument] attribute**: https://docs.rs/tracing/latest/tracing/attr.instrument.html
  - `skip_all` - don't log all arguments (for types that don't impl Debug)
  - `fields(...)` - explicitly declare fields
  - `err` - automatically log errors
- **Manual span creation**: https://docs.rs/tracing/latest/tracing/span/index.html
- **Span guards**: https://docs.rs/tracing/latest/tracing/span/struct.Span.html#method.enter

**Patterns:**
```rust
// Pattern 1: Attribute macro (easiest)
#[instrument(skip_all, fields(component = "my_service"))]
async fn my_async_fn() -> Result<()> {
    // Span created automatically
}

// Pattern 2: Manual span (more control)
let span = info_span!("operation", component = "my_service");
async move {
    // work
}.instrument(span).await

// Pattern 3: Sync code
let span = info_span!("operation");
let _guard = span.enter();
// work
```

### Tasks

#### 2.1 Coordinate with strata-common Repo

**Action Required:** Since `strata-service` is external, we need to:

1. **Open issue** in `alpenlabs/strata-common` proposing service framework instrumentation
2. **Draft PR** showing the changes
3. **Update dependency** in this repo once merged

**Issue Template:**

```markdown
# Proposal: Add Automatic Instrumentation to Service Framework

## Motivation
Currently, every service (ASM worker, CSM worker, etc.) must manually add tracing spans.
This is error-prone and leads to inconsistent observability.

By instrumenting the service framework itself, all services get observability automatically.

## Proposed Changes

1. Add `component_name()` method to `Service` trait
2. Instrument message handler in framework
3. Add span events for service lifecycle (launch, shutdown)

## Example

```rust
pub trait Service {
    type State;
    type Msg;
    type Status;

    // NEW: Each service declares its component name
    fn component_name() -> &'static str;

    fn get_status(state: &Self::State) -> Self::Status;
}

// Framework's message handler becomes:
pub fn handle_message<S: SyncService>(
    state: &mut S::State,
    msg: &S::Msg,
) -> anyhow::Result<Response> {
    let span = info_span!(
        "service_message",
        component = S::component_name(),
        otel.kind = "internal",
    );

    async move {
        let start = std::time::Instant::now();
        let result = S::process_input(state, msg);

        match &result {
            Ok(Response::Continue) => {
                info!(duration_ms = start.elapsed().as_millis(), "message processed");
            }
            Err(e) => {
                error!(error = %e, "message processing failed");
            }
        }

        result
    }.instrument(span).await
}
```

## Benefits
- All services get consistent instrumentation
- No manual instrumentation needed per service
- Automatic duration tracking
- Framework can add metrics in future (DORA, error rates, etc.)

## Breaking Changes
- Services must implement `component_name()` - but this is a one-liner per service
```

#### 2.2 Local Changes (After strata-common Update)

Once the framework change is merged, update each service:

**File:** `crates/asm/worker/src/service.rs`
**Owner:** Check `git blame crates/asm/worker/src/service.rs` (primary: Evgeny)

```rust
impl<W: WorkerContext + Send + Sync + 'static> Service for AsmWorkerService<W> {
    type State = AsmWorkerServiceState<W>;
    type Msg = L1BlockCommitment;
    type Status = AsmWorkerStatus;

    // NEW: Declare component name for instrumentation
    fn component_name() -> &'static str {
        "asm_worker"
    }

    fn get_status(state: &Self::State) -> Self::Status {
        AsmWorkerStatus {
            is_initialized: state.initialized,
            cur_block: state.blkid,
            cur_state: state.anchor.clone(),
        }
    }
}
```

**Repeat for:**
- CSM worker (if exists)
- Chain worker (if exists)
- Any other service implementations

#### 2.3 Verification

**After changes, all services should automatically log:**
```log
INFO service_message{component="asm_worker"}: strata_service: processing message
INFO service_message{component="asm_worker" duration_ms=123}: strata_service: message processed
```

**Verify with:**
```bash
RUST_LOG=info cargo run --bin strata-client 2>&1 | grep "service_message"
```

**Expected:** See automatic spans for every message processed by every service.

---

## Phase 3: Component-Level Instrumentation

**Objective:** Add fine-grained spans within service implementations to understand internal operations.

**Why Third:** Now we have framework-level instrumentation (Phase 2) as baseline. This phase adds detail *within* services to see what specific operations are slow.

**Dependency:** Phase 1 (span duration logging) and Phase 2 (framework baseline) must be complete.

**Effort:** 1-2 hours per component (incremental)

### Background Reading (Required)

#### 3.1 Span Hierarchy and Context

**Core Concept:** Spans form a tree. Child spans inherit parent's trace context.

**Read:**
- **Span Relationships**: https://opentelemetry.io/docs/specs/otel/trace/api/#span
  - Parent-child relationships
  - Span context propagation
- **Tracing Span Hierarchy**: https://docs.rs/tracing/latest/tracing/span/index.html#span-relationships

**Example Hierarchy:**
```
service_message (from Phase 2 framework)
  ├─ find_pivot_anchor
  │   ├─ get_l1_block
  │   └─ get_anchor_state
  └─ asm_transition (for each block)
      ├─ validate_block
      ├─ execute_state_transition
      └─ store_anchor_state
```

**Code Pattern:**
```rust
// Parent span (from framework in Phase 2)
fn process_input(...) -> Result<Response> {
    // Child span 1
    let pivot = {
        let span = debug_span!("find_pivot", component = "asm_worker");
        let _guard = span.enter();
        find_pivot_anchor()?
    };  // Span closes here, duration logged

    // Child span 2
    for block in blocks {
        let span = info_span!(
            "asm_transition",
            component = "asm_worker",
            l1_height = block.height(),
        );
        let _guard = span.enter();
        transition(block)?;
    }

    Ok(Response::Continue)
}
```

#### 3.2 When to Add Spans

**Heuristic:**
- **Always span:** Operations that cross crate boundaries, do I/O, or are performance-critical
- **Consider spanning:** Loops with variable iteration count, operations that might be slow
- **Don't span:** Simple getters, pure calculations, trivial operations

**Read:**
- **OpenTelemetry Span Best Practices**: https://opentelemetry.io/docs/specs/otel/trace/api/#span-operations
- **Performance Considerations**: https://github.com/tokio-rs/tracing/blob/master/tracing/benches/subscriber.rs

**Overhead Measurement:**
```rust
// Minimal span overhead: ~50-100ns per span
// Only matters if creating >10,000 spans/sec

// Use skip_all to avoid Debug formatting overhead
#[instrument(skip_all, fields(id = %item.id))]
fn process_item(item: &ComplexType) {
    // Only format fields you need
}
```

### Tasks

#### 3.1 Instrument ASM Worker

**File:** `crates/asm/worker/src/service.rs` (lines 37-104)
**Owner:** Evgeny (per git blame)

**Current Code:**
```rust
fn process_input(
    state: &mut AsmWorkerServiceState<W>,
    incoming_block: &L1BlockCommitment,
) -> anyhow::Result<Response> {
    // No instrumentation
    let pivot = find_pivot(...);
    for (block, block_id) in blocks {
        state.transition(block)?;
    }
    Ok(Response::Continue)
}
```

**Enhanced:**
```rust
#[instrument(
    skip_all,
    fields(
        component = "asm_worker",
        l1_height = incoming_block.height(),
        l1_block = %incoming_block.blkid(),  // Full block ID
    )
)]
fn process_input(
    state: &mut AsmWorkerServiceState<W>,
    incoming_block: &L1BlockCommitment,
) -> anyhow::Result<Response> {
    let ctx = &state.context;
    let genesis_height = state.params.rollup().genesis_l1_view.height();
    let height = incoming_block.height();

    if height < genesis_height {
        warn!(%height, "ignoring L1 block before genesis");
        return Ok(Response::Continue);
    }

    // Find pivot with sub-span
    let (pivot_block, skipped_blocks) = {
        let span = debug_span!("find_pivot", component = "asm_worker");
        let _guard = span.enter();

        let mut skipped_blocks = vec![];
        let mut pivot_block = *incoming_block;
        let mut pivot_anchor = ctx.get_anchor_state(&pivot_block);

        while pivot_anchor.is_err() && pivot_block.height() >= genesis_height {
            let block = ctx.get_l1_block(pivot_block.blkid())?;
            let parent_height = pivot_block.height().to_consensus_u32() - 1;
            let parent_block_id = L1BlockCommitment::from_height_u64(
                parent_height as u64,
                block.header.prev_blockhash.into(),
            )
            .expect("parent height should be valid");

            skipped_blocks.push((block, pivot_block));
            pivot_anchor = ctx.get_anchor_state(&parent_block_id);
            pivot_block = parent_block_id;
        }

        if pivot_block.height() < genesis_height {
            warn!("ASM hasn't found pivot anchor state at genesis");
            return Ok(Response::ShouldExit);
        }

        info!(%pivot_block, skipped_count = skipped_blocks.len(),
              "found pivot anchor state");
        (pivot_block, skipped_blocks)
    };  // find_pivot span closes here, duration logged

    let pivot_anchor = ctx.get_anchor_state(&pivot_block)?;
    state.update_anchor_state(pivot_anchor, pivot_block);

    // Process blocks with individual spans
    for (block, block_id) in skipped_blocks.iter().rev() {
        let _span = info_span!(
            "asm_transition",
            component = "asm_worker",
            l1_height = block_id.height(),
            l1_block = %block_id,
        ).entered();  // entered() returns guard

        info!("attempting state transition");

        match state.transition(block) {
            Ok(asm_stf_out) => {
                let new_state = AsmState::from_output(asm_stf_out);
                state.context.store_anchor_state(block_id, &new_state)?;
                state.update_anchor_state(new_state, *block_id);
                info!("state transition succeeded");
            }
            Err(e) => {
                error!(%e, "state transition failed");
                return Ok(Response::ShouldExit);
            }
        }
    }  // asm_transition span closes for each iteration

    Ok(Response::Continue)
}
```

**Key Changes:**
1. `#[instrument]` on function - captures l1_height and l1_block
2. `find_pivot` sub-span - measures pivot finding duration
3. `asm_transition` span per block - measures each transition separately
4. Full block IDs (not abbreviated) - `%incoming_block.blkid()` not `..` format
5. Structured fields - `l1_height`, `l1_block`, `component`

**References:**
- #[instrument]: https://docs.rs/tracing/latest/tracing/attr.instrument.html
- debug_span!: https://docs.rs/tracing/latest/tracing/macro.debug_span.html
- .entered(): https://docs.rs/tracing/latest/tracing/span/struct.Span.html#method.entered

#### 3.2 Instrument Other Critical Paths

**Priority Order (based on critical path):**

1. **CSM Worker** (checkpoint state manager) - if exists
2. **Fork Choice Manager** - block validation and selection
3. **Block Template Generation** - sequencer block building
4. **Proof Generation** - prover service workflow

**For each component:**
- Find the main processing function
- Add `#[instrument]` with component field
- Add sub-spans for major operations (validation, I/O, state changes)
- Use full identifiers, not abbreviated

**Standard Template:**
```rust
#[instrument(
    skip_all,
    fields(
        component = "<component_name>",
        // Add relevant entity IDs
    )
)]
fn process_something(...) -> Result<()> {
    // Sub-operation spans
    let result = {
        let _span = debug_span!("sub_operation", component = "<component_name>").entered();
        expensive_operation()?
    };

    Ok(())
}
```

#### 3.3 Verification

**Test 1: Span Hierarchy**
```bash
RUST_LOG=debug cargo run --bin strata-client 2>&1 | grep -E "(service_message|find_pivot|asm_transition)"

# Expected output showing hierarchy:
# INFO service_message{component="asm_worker" l1_height=100}: processing message
# DEBUG find_pivot{component="asm_worker"}: close time.busy=5ms
# INFO asm_transition{component="asm_worker" l1_height=99}: attempting state transition
# INFO asm_transition{component="asm_worker" l1_height=99}: close time.busy=123ms
# INFO asm_transition{component="asm_worker" l1_height=100}: attempting state transition
# INFO asm_transition{component="asm_worker" l1_height=100}: close time.busy=98ms
# INFO service_message{component="asm_worker"}: close time.busy=226ms
```

**Test 2: Filter by Component**
```bash
# See all ASM worker activity
grep 'component="asm_worker"' service.log

# See timing for specific operations
grep 'asm_transition.*close' service.log | awk '{print $NF}'
```

**Test 3: Performance Overhead**
```bash
# Before instrumentation
time cargo run --bin strata-client -- <benchmark command>

# After instrumentation
time cargo run --bin strata-client -- <benchmark command>

# Overhead should be < 3% CPU time
```

---

## Phase 4: Cross-Service Trace Context

**Objective:** Propagate W3C trace context across RPC boundaries so distributed operations share a trace ID.

**Why Fourth:** We now have spans within services (Phase 3). This phase connects those spans across services so you can see the full distributed trace.

**Dependency:** Phase 1 (TraceContextPropagator setup) must be complete. Phase 3 should be done for at least 2 services you want to trace between.

**Effort:** 2-3 days

### Background Reading (Required)

#### 4.1 Distributed Tracing Concepts

**Core Problem:** When Service A calls Service B via RPC, their spans are disconnected. We need to link them.

**Read:**
- **Distributed Tracing**: https://opentelemetry.io/docs/concepts/signals/traces/#spans-in-opentelemetry
  - SpanContext: Contains trace_id, span_id, trace_flags
  - Context Propagation: How SpanContext flows between services
- **W3C Trace Context**: https://www.w3.org/TR/trace-context/#design-overview
  - traceparent header format
  - How to maintain parent-child relationships across processes

**How It Works:**
```
[Service A]                      [Service B]
   span_a (trace_id=123)
     ↓ RPC call
     ├─ inject trace_id=123 into headers
     └→ HTTP request
                                    ↓
                                 extract trace_id=123 from headers
                                    ↓
                                 span_b (trace_id=123, parent=span_a)
```

Both spans share trace_id=123, and span_b knows its parent is span_a.

**Reference:** https://opentelemetry.io/docs/specs/otel/context/

#### 4.2 jsonrpsee and Context Propagation

**Challenge:** jsonrpsee doesn't natively support arbitrary HTTP headers in JSON-RPC.

**Options:**
1. **Include trace context in JSON-RPC params** (simplest)
2. **Use HTTP headers** (requires jsonrpsee middleware)
3. **Custom transport** (most flexible, most work)

**Read:**
- **jsonrpsee Custom Middleware**: https://docs.rs/jsonrpsee/latest/jsonrpsee/server/middleware/index.html
- **Adding custom params**: https://github.com/paritytech/jsonrpsee/blob/master/examples/examples/http.rs

**Recommendation:** Option 1 (params) for initial implementation - it's backward compatible and simple.

**Pattern:**
```rust
// Add optional trace_ctx parameter to RPC methods
#[method(name = "getBlocks")]
async fn get_blocks(
    &self,
    idx: u64,
    #[serde(default)] trace_ctx: Option<TraceContext>,  // Optional for backward compat
) -> RpcResult<Vec<Block>>;
```

#### 4.3 Creating Trace Context Module

**Read:**
- **OpenTelemetry Baggage**: https://opentelemetry.io/docs/specs/otel/baggage/api/
  - Key-value pairs that propagate with trace
- **Span Context**: https://docs.rs/opentelemetry/latest/opentelemetry/trace/struct.SpanContext.html

### Tasks

#### 4.1 Create TraceContext Module

**New File:** `crates/common/src/tracing_context.rs`

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Trace context that flows across service boundaries
///
/// This is included in RPC calls to maintain distributed trace continuity.
/// See: https://www.w3.org/TR/trace-context/
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TraceContext {
    /// Unique request ID for correlation (UUID v4)
    pub request_id: String,

    /// W3C traceparent header
    /// Format: "00-{trace-id}-{parent-id}-{flags}"
    /// See: https://www.w3.org/TR/trace-context/#traceparent-header
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,

    /// W3C tracestate header (vendor-specific data)
    /// See: https://www.w3.org/TR/trace-context/#tracestate-header
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}

impl TraceContext {
    /// Create a new root trace context (starts a new trace)
    pub fn new_root() -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            traceparent: None,
            tracestate: None,
        }
    }

    /// Extract trace context from current tracing span
    ///
    /// Reads OpenTelemetry context from current span and encodes it
    /// as W3C traceparent header for propagation.
    pub fn from_current_span() -> Self {
        let request_id = Uuid::new_v4().to_string();
        let (traceparent, tracestate) = extract_otel_context();

        Self {
            request_id,
            traceparent,
            tracestate,
        }
    }

    /// Get short request ID for logging (first 8 chars)
    pub fn short_id(&self) -> &str {
        &self.request_id[..8.min(self.request_id.len())]
    }
}

/// Extract OpenTelemetry context from current span as W3C headers
fn extract_otel_context() -> (Option<String>, Option<String>) {
    use opentelemetry::global;
    use opentelemetry::propagation::TextMapPropagator;
    use std::collections::HashMap;

    let propagator = global::get_text_map_propagator(|p| p.clone());
    let context = tracing::Span::current().context();

    let mut carrier = HashMap::new();
    propagator.inject_context(&context, &mut carrier);

    (
        carrier.get("traceparent").cloned(),
        carrier.get("tracestate").cloned(),
    )
}

/// Inject trace context into current span
///
/// Takes W3C traceparent/tracestate headers and sets them as parent
/// of the current span, linking distributed traces.
///
/// See: https://docs.rs/opentelemetry/latest/opentelemetry/trace/trait.Span.html#tymethod.set_parent
pub fn inject_trace_context(ctx: &TraceContext) {
    use opentelemetry::global;
    use opentelemetry::propagation::TextMapPropagator;
    use std::collections::HashMap;

    if let Some(ref traceparent) = ctx.traceparent {
        let mut carrier = HashMap::new();
        carrier.insert("traceparent".to_string(), traceparent.clone());
        if let Some(ref tracestate) = ctx.tracestate {
            carrier.insert("tracestate".to_string(), tracestate.clone());
        }

        let propagator = global::get_text_map_propagator(|p| p.clone());
        let parent_ctx = propagator.extract(&carrier);

        // Link current span to distributed parent
        tracing::Span::current().set_parent(parent_ctx);
    }
}
```

**Add to `crates/common/src/lib.rs`:**
```rust
pub mod tracing_context;
```

**References:**
- Uuid: https://docs.rs/uuid/latest/uuid/struct.Uuid.html
- TextMapPropagator: https://docs.rs/opentelemetry/latest/opentelemetry/propagation/trait.TextMapPropagator.html
- Span.set_parent: https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/trait.OpenTelemetrySpanExt.html

#### 4.2 Update RPC API Signatures

**Strategy:** Add `trace_ctx` as optional last parameter. Backward compatible because `#[serde(default)]` makes it optional in JSON-RPC.

**File:** `crates/rpc/api/src/lib.rs` (or equivalent)

```rust
use strata_common::tracing_context::TraceContext;

#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client"), rpc(server, client, namespace = "strata"))]
pub trait StrataApi {
    /// Get blocks at a certain height
    #[method(name = "getBlocksAtIdx")]
    async fn get_blocks_at_idx(
        &self,
        idx: u64,
        #[serde(default)] trace_ctx: Option<TraceContext>,
    ) -> RpcResult<Vec<HexBytes32>>;

    // Add to all critical RPC methods:
    // - get_block_template
    // - complete_block_template
    // - submit_checkpoint_proof
    // - etc.
}
```

**Why Optional:** Older clients not sending trace_ctx will pass `None` automatically. New clients pass `Some(ctx)`.

**Reference:** https://docs.rs/jsonrpsee/latest/jsonrpsee/proc_macros/attr.rpc.html

#### 4.3 Update RPC Server Handlers

**Pattern for all handlers:**
1. Extract or create trace context
2. Inject into current span
3. Create span with req_id
4. Process request

**Example:** `bin/strata-client/src/rpc_server.rs`

```rust
use strata_common::tracing_context::{TraceContext, inject_trace_context};

#[async_trait]
impl StrataApiServer for StrataRpcImpl {
    async fn get_blocks_at_idx(
        &self,
        idx: u64,
        trace_ctx: Option<TraceContext>,
    ) -> RpcResult<Vec<HexBytes32>> {
        // 1. Get or create trace context
        let trace_ctx = trace_ctx.unwrap_or_else(TraceContext::new_root);

        // 2. Inject into current span (links to caller if traceparent present)
        inject_trace_context(&trace_ctx);

        // 3. Create span with req_id for correlation
        let span = info_span!(
            "rpc_handler",
            component = "strata_rpc",
            rpc_method = "getBlocksAtIdx",
            req_id = trace_ctx.short_id(),
            otel.kind = "server",  // OpenTelemetry semantic convention
            idx,
        );

        async move {
            info!(idx, "handling getBlocksAtIdx request");

            self.storage
                .l2()
                .get_blocks_at_slot(idx)
                .await
                .map(|blocks| blocks.into_iter().map(HexBytes32).collect())
                .map_err(to_jsonrpsee_error)
        }
        .instrument(span)
        .await
    }

    // Repeat pattern for all RPC methods
}
```

**Key Points:**
- `inject_trace_context` links this span to caller's span
- `req_id` provides correlation even if OpenTelemetry isn't set up
- `otel.kind = "server"` follows OpenTelemetry semantic conventions

**References:**
- .instrument(): https://docs.rs/tracing/latest/tracing/trait.Instrument.html
- OpenTelemetry Span Kind: https://opentelemetry.io/docs/specs/semconv/trace/span-kind/

#### 4.4 Update RPC Clients

**Pattern for all client calls:**
1. Extract trace context from current span
2. Pass it in RPC call

**Example:** Sequencer calling Reth Engine API

```rust
use strata_common::tracing_context::TraceContext;

async fn get_block_template(&self, config: BlockGenerationConfig) -> Result<BlockTemplate> {
    // Extract trace context from current span
    let trace_ctx = TraceContext::from_current_span();

    // Pass in RPC call
    self.rpc_client
        .get_payload_v4(payload_id, Some(trace_ctx))
        .await
}
```

**For jsonrpsee client calls:**
```rust
// If you control both client and server:
let result: BlockTemplate = client
    .request("engine_getPayloadV4", rpc_params![payload_id, trace_ctx])
    .await?;
```

#### 4.5 Verification

**Test 1: Trace Context Propagation**
```bash
# Start both services with OTLP enabled
RUST_LOG=info STRATA_OTLP_URL=http://localhost:4317 cargo run --bin strata-client &
RUST_LOG=info STRATA_OTLP_URL=http://localhost:4317 cargo run --bin prover-client &

# Make a cross-service RPC call
# Check logs for same req_id appearing in both services:

# Service A (client):
# INFO rpc_call{component="prover_client" req_id="a1b2c3d4"}: calling submitCheckpointProof

# Service B (server):
# INFO rpc_handler{component="strata_rpc" req_id="a1b2c3d4"}: handling submitCheckpointProof
```

**Test 2: Verify in Grafana Tempo**

If you have Grafana Tempo set up:
```
# Query by trace ID (should show spans from multiple services)
{trace_id="4bf92f3577b34da6a3ce929d0e0e4736"}

# Should see service graph showing:
prover-client → strata-client → reth
```

**Test 3: Grep Logs by req_id**
```bash
# Grep all logs for a single request
grep "req_id=\"a1b2c3d4\"" */service.log

# Should see timeline across services:
# 10:00:00.000 prover-client: starting checkpoint proof
# 10:00:00.100 strata-client: received submitCheckpointProof RPC
# 10:00:00.150 strata-client: checkpoint verified
# 10:00:00.200 prover-client: checkpoint submission complete
```

---

## Phase 5: Security & Operational Metrics

**Objective:** Define and instrument security-critical metrics for incident response, anomaly detection, and audit capability.

**Why Fifth:** We now have comprehensive tracing (Phases 1-4). This phase adds *metrics* for monitoring trends, alerting on anomalies, and security analysis.

**Dependency:** Phase 3 (component instrumentation) should be complete so we know where to add metric recording.

**Effort:** 2-3 days

### Background Reading (Required)

#### 5.1 Metrics vs Traces vs Logs

**Read:**
- **OpenTelemetry Signals**: https://opentelemetry.io/docs/concepts/signals/
  - Traces: Request-scoped, distributed
  - Metrics: Aggregated, time-series
  - Logs: Discrete events
- **When to use what**: https://www.honeycomb.io/blog/metrics-logs-and-traces-the-golden-triangle

**Traces vs Metrics:**
- **Traces:** "Show me the path request X took" (high cardinality, sampled)
- **Metrics:** "Show me error rate trend over 24h" (aggregated, continuous)

**Security Needs Metrics For:**
- **Alerting:** High error rate, unexpected patterns
- **Trending:** Is attack surface increasing?
- **Dashboards:** Real-time health visualization
- **Compliance:** Audit trail of security events

#### 5.2 DORA Metrics

**Read:**
- **DORA Research**: https://dora.dev/
- **Four Key Metrics**:
  1. Deployment Frequency
  2. Lead Time for Changes
  3. Change Failure Rate
  4. Time to Restore Service

**Why DORA for Security (Sam's point):**
- High change failure rate = potential security regressions
- Long time to restore = vulnerability window
- Tracking these helps correlate security incidents with deployments

#### 5.3 Metrics in Rust with tracing

**Read:**
- **tracing-subscriber metrics**: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/layer/trait.Layer.html#method.on_event
  - Can extract metrics from tracing events
- **metrics crate**: https://docs.rs/metrics/latest/metrics/
  - Prometheus-style counters, gauges, histograms
- **Integration**: https://github.com/tokio-rs/tracing/tree/master/tracing-subscriber#metrics

**Pattern (trey's suggestion):**
```rust
// Use tracing events to drive metrics
#[instrument]
fn process_checkpoint(proof: &Proof) -> Result<()> {
    let result = verify_proof(proof);

    match result {
        Ok(()) => {
            info!("checkpoint verified");
            // Metrics extracted from this event by subscriber layer
        }
        Err(e) => {
            error!("checkpoint verification failed");
            // Security metric recorded automatically
        }
    }

    result
}
```

### Tasks

#### 5.1 Define Security-Critical Metrics

**Based on Sam's feedback**, we need metrics for:

1. **Incident Response:**
   - RPC failure rates (by method)
   - Proof verification failure rate
   - Block validation failure rate
   - Unexpected state transitions

2. **Anomaly Detection:**
   - RPC latency p99 (outliers indicate issues)
   - Proof generation time distribution
   - Checkpoint submission retry count
   - State divergence events

3. **Audit Capability:**
   - Total checkpoints submitted
   - Total proofs verified
   - Slash events (if applicable)
   - Admin operations (config changes)

**Create:** `crates/common/src/metrics_definitions.rs`

```rust
//! Security and operational metrics definitions
//!
//! These metrics are extracted from tracing events and exported to Prometheus.
//! See: https://opentelemetry.io/docs/specs/semconv/general/metrics/

use metrics::{counter, histogram, gauge, describe_counter, describe_histogram, describe_gauge};

/// Initialize metric descriptions (call at startup)
pub fn init_metrics() {
    // Incident Response Metrics
    describe_counter!(
        "strata_rpc_failures_total",
        "Total RPC call failures by method"
    );

    describe_counter!(
        "strata_proof_verification_failures_total",
        "Total proof verification failures"
    );

    describe_counter!(
        "strata_block_validation_failures_total",
        "Total block validation failures"
    );

    describe_counter!(
        "strata_unexpected_transitions_total",
        "Total unexpected state transitions (security signal)"
    );

    // Anomaly Detection Metrics
    describe_histogram!(
        "strata_rpc_duration_seconds",
        "RPC call duration by method"
    );

    describe_histogram!(
        "strata_proof_generation_seconds",
        "Proof generation duration"
    );

    describe_counter!(
        "strata_checkpoint_submission_retries_total",
        "Total checkpoint submission retries"
    );

    // Audit Metrics
    describe_counter!(
        "strata_checkpoints_submitted_total",
        "Total checkpoints submitted to L1"
    );

    describe_counter!(
        "strata_proofs_verified_total",
        "Total proofs verified"
    );

    describe_counter!(
        "strata_slash_events_total",
        "Total slash events (security critical)"
    );

    describe_counter!(
        "strata_admin_operations_total",
        "Total admin operations by type"
    );

    // DORA Metrics
    describe_counter!(
        "strata_deployments_total",
        "Total deployments (track deployment frequency)"
    );

    describe_histogram!(
        "strata_recovery_duration_seconds",
        "Time to recover from failures"
    );
}

/// Record RPC failure
pub fn record_rpc_failure(method: &str, error_type: &str) {
    counter!("strata_rpc_failures_total", "method" => method.to_string(), "error_type" => error_type.to_string()).increment(1);
}

/// Record RPC duration
pub fn record_rpc_duration(method: &str, duration_secs: f64) {
    histogram!("strata_rpc_duration_seconds", "method" => method.to_string()).record(duration_secs);
}

/// Record proof verification failure
pub fn record_proof_verification_failure(reason: &str) {
    counter!("strata_proof_verification_failures_total", "reason" => reason.to_string()).increment(1);
}

/// Record unexpected state transition (security signal)
pub fn record_unexpected_transition(from_state: &str, to_state: &str) {
    counter!(
        "strata_unexpected_transitions_total",
        "from" => from_state.to_string(),
        "to" => to_state.to_string()
    ).increment(1);
}

/// Record checkpoint submission
pub fn record_checkpoint_submitted(checkpoint_index: u64) {
    counter!("strata_checkpoints_submitted_total").increment(1);
    gauge!("strata_latest_checkpoint_index").set(checkpoint_index as f64);
}

/// Record slash event (security critical)
pub fn record_slash_event(reason: &str, sequencer: &str) {
    counter!(
        "strata_slash_events_total",
        "reason" => reason.to_string(),
        "sequencer" => sequencer.to_string()
    ).increment(1);
}
```

**References:**
- metrics crate: https://docs.rs/metrics/latest/metrics/
- Prometheus metric types: https://prometheus.io/docs/concepts/metric_types/
- OpenTelemetry semantic conventions: https://opentelemetry.io/docs/specs/semconv/

#### 5.2 Instrument Critical Security Points

**Pattern:** Add metric recording alongside error logging.

**Example 1: RPC Failure Tracking**

**File:** RPC server handler (from Phase 4.3)

```rust
use strata_common::metrics_definitions;

async fn get_blocks_at_idx(
    &self,
    idx: u64,
    trace_ctx: Option<TraceContext>,
) -> RpcResult<Vec<HexBytes32>> {
    let trace_ctx = trace_ctx.unwrap_or_else(TraceContext::new_root);
    inject_trace_context(&trace_ctx);

    let span = info_span!(
        "rpc_handler",
        component = "strata_rpc",
        rpc_method = "getBlocksAtIdx",
        req_id = trace_ctx.short_id(),
    );

    let start = std::time::Instant::now();

    let result = async move {
        info!(idx, "handling getBlocksAtIdx request");

        self.storage
            .l2()
            .get_blocks_at_slot(idx)
            .await
            .map(|blocks| blocks.into_iter().map(HexBytes32).collect())
            .map_err(to_jsonrpsee_error)
    }
    .instrument(span)
    .await;

    // Record metrics
    let duration = start.elapsed().as_secs_f64();
    match &result {
        Ok(_) => {
            metrics_definitions::record_rpc_duration("getBlocksAtIdx", duration);
        }
        Err(e) => {
            let error_type = classify_error(e);  // You define this
            error!(error = %e, error_type = %error_type, "RPC call failed");
            metrics_definitions::record_rpc_failure("getBlocksAtIdx", error_type);
            metrics_definitions::record_rpc_duration("getBlocksAtIdx", duration);
        }
    }

    result
}
```

**Example 2: Proof Verification Tracking**

**File:** Wherever checkpoint proofs are verified

```rust
use strata_common::metrics_definitions;

fn verify_checkpoint_proof(proof: &CheckpointProof) -> Result<()> {
    match proof.verify() {
        Ok(()) => {
            info!(checkpoint_index = proof.index, "proof verified");
            metrics_definitions::record_proof_verified(proof.index);
            Ok(())
        }
        Err(e) => {
            let reason = format!("{:?}", e);
            error!(
                checkpoint_index = proof.index,
                error = %e,
                "proof verification failed"
            );
            metrics_definitions::record_proof_verification_failure(&reason);
            Err(e)
        }
    }
}
```

**Example 3: Slash Event Tracking**

**File:** Wherever slash transactions are processed

```rust
fn process_slash_tx(slash: &SlashTx) -> Result<()> {
    info!(
        sequencer = %slash.sequencer_id,
        reason = %slash.reason,
        "processing slash transaction"
    );

    metrics_definitions::record_slash_event(
        &slash.reason,
        &slash.sequencer_id.to_string()
    );

    // Process slash...
    Ok(())
}
```

#### 5.3 Export Metrics via Prometheus

**Add to service binary** (e.g., `bin/strata-client/src/main.rs`):

```rust
use metrics_exporter_prometheus::PrometheusBuilder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize metrics
    strata_common::metrics_definitions::init_metrics();

    // Start Prometheus exporter on port 9090
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9090))
        .install()
        .expect("failed to install Prometheus exporter");

    info!("Prometheus metrics available at :9090/metrics");

    // ... rest of service initialization
}
```

**Verify:**
```bash
cargo run --bin strata-client &
curl http://localhost:9090/metrics

# Should see:
# # HELP strata_rpc_failures_total Total RPC call failures by method
# # TYPE strata_rpc_failures_total counter
# strata_rpc_failures_total{method="getBlocksAtIdx",error_type="timeout"} 0
# ...
```

**References:**
- metrics-exporter-prometheus: https://docs.rs/metrics-exporter-prometheus/latest/metrics_exporter_prometheus/
- Prometheus scraping: https://prometheus.io/docs/prometheus/latest/configuration/configuration/#scrape_config

#### 5.4 Grafana Dashboards & Alerts

**Dashboard Panels:**

1. **Security Overview**
   - Panel: Total failures (counter)
   - Panel: Failure rate by method (rate[5m])
   - Panel: Proof verification failures (counter)
   - Panel: Slash events (counter with labels)

2. **Performance**
   - Panel: RPC latency p50/p95/p99 (histogram quantiles)
   - Panel: Proof generation time (histogram)
   - Panel: Request rate by method

3. **DORA Metrics**
   - Panel: Deployment frequency (counter over time)
   - Panel: Change failure rate (failures / deployments)
   - Panel: Mean time to recovery (histogram avg)

**Alert Rules (PromQL):**

```yaml
# alert-rules.yml
groups:
  - name: security
    rules:
      # High RPC failure rate
      - alert: HighRPCFailureRate
        expr: |
          rate(strata_rpc_failures_total[5m])
          /
          rate(strata_rpc_calls_total[5m]) > 0.05
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High RPC failure rate ({{ $value | humanizePercentage }})"

      # Any proof verification failure
      - alert: ProofVerificationFailure
        expr: increase(strata_proof_verification_failures_total[5m]) > 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Proof verification failed"

      # Slash event occurred
      - alert: SlashEventDetected
        expr: increase(strata_slash_events_total[5m]) > 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Slash event detected: {{ $labels.reason }}"

      # Slow RPC calls
      - alert: SlowRPCCalls
        expr: |
          histogram_quantile(0.99,
            rate(strata_rpc_duration_seconds_bucket[5m])
          ) > 5.0
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "p99 RPC latency above 5s"
```

**References:**
- Prometheus alerting: https://prometheus.io/docs/prometheus/latest/configuration/alerting_rules/
- Grafana dashboards: https://grafana.com/docs/grafana/latest/dashboards/
- PromQL examples: https://prometheus.io/docs/prometheus/latest/querying/examples/

#### 5.5 Verification

**Test 1: Metrics Export**
```bash
curl http://localhost:9090/metrics | grep strata_

# Verify all defined metrics appear
```

**Test 2: Trigger Metric Recording**
```bash
# Cause an RPC failure
# Check metric increased:
curl http://localhost:9090/metrics | grep "strata_rpc_failures_total"

# strata_rpc_failures_total{method="getBlocksAtIdx",error_type="timeout"} 1
```

**Test 3: Grafana Visualization**

If you have Grafana + Prometheus set up:
1. Import dashboard from `dashboards/security-overview.json` (you'll create this)
2. Verify panels show real-time data
3. Test alert by causing a failure

---

## Phase 6: Logging Standards & Enforcement

**Objective:** Standardize field naming, eliminate abbreviated identifiers, enforce conventions via code review and tooling.

**Why Last:** This is ongoing enforcement. Phases 1-5 establish the infrastructure and patterns. This phase ensures consistency as the codebase evolves.

**Effort:** Initial: 1 day (create standards doc, initial cleanup). Ongoing: Code review enforcement.

### Tasks

#### 6.1 Document Logging Standards

**Create:** `docs/observability-standards.md`

```markdown
# Observability Standards

## Mandatory Span Fields

Every span MUST include:
- `component = "<logical_component>"` - Logical subsystem (asm_worker, fork_choice, etc.)
- Entity IDs relevant to the operation (see below)

## Field Naming Conventions

### L1 Entities
- `l1_height = <u64>` - L1 block height
- `l1_block = %<L1BlockId>` - **FULL** L1 block ID (NOT abbreviated)

### L2 Entities
- `l2_slot = <u64>` - L2 slot number
- `l2_block = %<L2BlockId>` - **FULL** L2 block ID
- `l2_tx = %<TxHash>` - Transaction hash

### Checkpoints
- `checkpoint_index = <u64>` - Checkpoint index
- `checkpoint_hash = %<Hash>` - Checkpoint hash

### Trace Correlation
- `req_id = <String>` - Request ID for cross-service correlation (from TraceContext)

## ❌ DON'T: Abbreviated Identifiers

```rust
// BAD - ungrepable
info!("processing block {}@{}..{}", height, &blkid[..4], &blkid[blkid.len()-4..]);
```

## ✅ DO: Full Identifiers

```rust
// GOOD - grepable
info!(
    component = "asm_worker",
    l1_height = height,
    l1_block = %blkid,  // Full hash
    "processing L1 block"
);
```

## Error Logging Requirements

Every error MUST include:
1. What failed (operation name)
2. What we were processing (entity IDs)
3. Why it failed (error message)
4. Component context

```rust
match state.transition(block) {
    Ok(output) => output,
    Err(e) => {
        error!(
            error = %e,
            component = "asm_worker",
            l1_height = block_id.height(),
            l1_block = %block_id,
            operation = "asm_transition",
            "ASM state transition failed"
        );
        return Err(e);
    }
}
```

## OpenTelemetry Semantic Conventions

When applicable, use standard OpenTelemetry fields:
- `otel.kind` - "client", "server", "internal", "producer", "consumer"
- `rpc.method` - RPC method name
- `rpc.service` - RPC service name

See: https://opentelemetry.io/docs/specs/semconv/

## Component Names

Maintain this list of canonical component names:

- `asm_worker` - ASM service worker
- `csm_worker` - CSM service worker
- `chain_worker` - Chain state worker
- `fork_choice` - Fork choice manager
- `strata_rpc` - RPC server
- `prover_client` - Prover service
- `btcio` - Bitcoin I/O layer
- (add more as needed)
```

#### 6.2 Initial Cleanup

**Find and fix abbreviated identifiers:**

```bash
# Find abbreviated block IDs
rg '(\.\.|truncate|[0-9]+@[a-f0-9]{4}\.\.[a-f0-9]{4})' --type rust

# Find log macros without structured fields
rg 'info!\("' --type rust | grep -v 'component ='

# Find spans without component field
rg '#\[instrument' --type rust -A3 | grep -v 'component ='
```

**For each violation:**
- Rewrite to use structured logging
- Use full identifiers
- Add component field

#### 6.3 Enforcement via CI

**Add to `.github/workflows/observability-check.yml`:**

```yaml
name: Observability Standards Check

on: [pull_request]

jobs:
  check-logging-standards:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Check for abbreviated block IDs
        run: |
          if rg '\.\..*blk|blkid.*\.\.' --type rust; then
            echo "❌ Found abbreviated block IDs. Use full identifiers."
            exit 1
          fi

      - name: Check for spans without component field
        run: |
          # This is a heuristic - may need refinement
          violations=$(rg '#\[instrument' --type rust -A3 | grep -v 'component =' | wc -l)
          if [ "$violations" -gt 10 ]; then
            echo "⚠️  Found $violations spans without component field"
            echo "See docs/observability-standards.md"
          fi

      - name: Check for unstructured logging in hot paths
        run: |
          # Find info!() calls with string formatting instead of structured fields
          if rg 'info!\(".*\{.*\}' --type rust crates/asm crates/csm; then
            echo "⚠️  Found unstructured logging. Use structured fields instead."
            echo "See docs/observability-standards.md"
          fi
```

**Reference:** https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions

#### 6.4 Code Review Checklist

**Add to `CONTRIBUTING.md`:**

```markdown
## Observability Code Review Checklist

When reviewing code, check:

- [ ] All spans have `component` field
- [ ] Entity IDs (block, tx, checkpoint) use full identifiers, not abbreviated
- [ ] Errors include: what failed, entity IDs, error message, component
- [ ] No string formatting in log messages (use structured fields instead)
- [ ] Cross-service RPC calls propagate TraceContext
- [ ] Critical operations have spans (I/O, state changes, validation)
- [ ] Span names follow conventions (snake_case, descriptive)

See: `docs/observability-standards.md`
```

#### 6.5 Tooling: Auto-Formatter

**Optional:** Create a tool to auto-fix common violations.

**Example:** `scripts/fix-logging.sh`

```bash
#!/bin/bash
# Auto-fix common logging violations

# Replace abbreviated block ID pattern
fd -e rs --exec sed -i 's/info!(".*{}@{}\.\.\({}\|[^}]*\)"/info!(l1_height = height, l1_block = %blkid, "processing block")/g'

# Note: This is fragile. Better to catch in review.
```

---

## Implementation Summary

### Phase Order & Dependencies

```
Phase 1: OTLP Foundation
   ↓
Phase 2: Service Framework
   ↓
Phase 3: Component Instrumentation (learn patterns, measure overhead)
   ↓
Phase 4: Cross-Service Trace Context (connect distributed operations)
   ↓
Phase 5: Security Metrics (monitor trends, alert on anomalies)
   ↓
Phase 6: Standards & Enforcement (maintain consistency)
```

### Effort Estimates

| Phase | Effort | Blocking? |
|-------|--------|-----------|
| 1. OTLP Foundation | 1 day | Yes - blocks everything |
| 2. Service Framework | 2 days | Yes - high leverage |
| 3. Component Instrumentation | 1-2 hours per component | No - incremental |
| 4. Cross-Service Trace | 2-3 days | Partially - needs Phase 3 done for 2+ services |
| 5. Security Metrics | 2-3 days | No - parallel with Phase 3/4 |
| 6. Standards | 1 day + ongoing | No - enforcement layer |

**Total:** ~2 weeks with 1-2 engineers

### Success Criteria

- [ ] Phase 1: Span durations logged automatically, OTLP exporting to Grafana
- [ ] Phase 2: All services get automatic `service_message` spans
- [ ] Phase 3: 80% of critical operations have spans with `component` field
- [ ] Phase 4: Cross-service RPC calls show same `req_id` in both services
- [ ] Phase 5: Security metrics exported to Prometheus, alerts configured
- [ ] Phase 6: CI checks pass, no abbreviated identifiers in new code

### Verification Commands

```bash
# Phase 1: Span duration logging works
RUST_LOG=info cargo run --bin strata-client 2>&1 | grep "close time.busy"

# Phase 2: Service framework instrumentation works
grep "service_message.*component=" service.log

# Phase 3: Components have spans
grep 'component="asm_worker"' service.log

# Phase 4: Cross-service trace works
grep 'req_id="abc12345"' */service.log

# Phase 5: Metrics exported
curl http://localhost:9090/metrics | grep strata_

# Phase 6: Standards enforced
rg '\.\.' --type rust crates/ | wc -l  # Should be 0 (or very low)
```

---

## References

### Core Documentation
- **OpenTelemetry**: https://opentelemetry.io/docs/
- **Tracing**: https://docs.rs/tracing/
- **Tracing Subscriber**: https://docs.rs/tracing-subscriber/
- **W3C Trace Context**: https://www.w3.org/TR/trace-context/
- **Metrics**: https://docs.rs/metrics/

### Examples & Guides
- **Tokio Tracing Blog**: https://tokio.rs/blog/2019-08-tracing
- **Tracing Guide**: https://github.com/tokio-rs/tracing/tree/master/examples
- **OpenTelemetry Rust Examples**: https://github.com/open-telemetry/opentelemetry-rust/tree/main/examples
- **Distributed Tracing in Practice**: https://peter.bourgon.org/blog/2017/02/21/metrics-tracing-and-logging.html

### Specifications
- **OpenTelemetry Trace Spec**: https://opentelemetry.io/docs/specs/otel/trace/
- **OpenTelemetry Semantic Conventions**: https://opentelemetry.io/docs/specs/semconv/
- **Prometheus Metric Types**: https://prometheus.io/docs/concepts/metric_types/
- **DORA Metrics**: https://dora.dev/

---

## Next Steps

1. **Review this document** with team
2. **Start with Phase 1** - Foundation work (blocks everything else)
3. **Coordinate Phase 2** with strata-common repo owners
4. **Assign component owners** for Phase 3 incremental work
5. **Set up Grafana/Prometheus** if not already done (for Phase 5 verification)
6. **Create tracking issues** for each phase
