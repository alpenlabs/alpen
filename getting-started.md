# Getting Started with Tracing: From Logging to Subscribers

**Author:** Engineering Team
**Date:** 2025-12-03
**Purpose:** Explain the difference between logging and tracing, and why the subscriber model matters

---

## Table of Contents

1. [Logging vs Tracing: The Fundamental Difference](#logging-vs-tracing-the-fundamental-difference)
2. [The Subscriber Model: One Instrumentation, Many Consumers](#the-subscriber-model-one-instrumentation-many-consumers)
3. [Key Concepts: Spans, Events, and Subscribers](#key-concepts-spans-events-and-subscribers)
4. [Why Subscribers Matter More Than Verbosity](#why-subscribers-matter-more-than-verbosity)
5. [Distributed Tracing Explained](#distributed-tracing-explained)
6. [Why This Matters for Vertex Core](#why-this-matters-for-vertex-core)
7. [Reading Order](#reading-order)

---

## Logging vs Tracing: The Fundamental Difference

### Traditional Logging: Hard-Coded Output

**How it works:**

```rust
// You write log statements
log::info!("Processing block {}", height);
log::error!("Operation failed: {}", error);

// They go to one place: stdout/file
// That's it. Done.
```

**The limitation:**
- **Single consumer:** Logs go to one destination (stdout, file)
- **Fixed format:** You decide the format when you write the code
- **Can't change without redeploying:** Want JSON instead of text? Change code, redeploy
- **Single purpose:** Logs are for humans reading text files

**Mental model:** Logging is like `println!` debugging that survived into production.

---

### Tracing: Emit Events, Let Subscribers Decide What to Do

**How it works:**

```rust
// You emit structured events with spans
let span = info_span!("process_block", l1_height = height, component = "worker");
let _enter = span.enter();

info!("starting validation");
validate_block()?;
info!("validation complete");

// Multiple subscribers consume these events:
// - Subscriber 1: Formats and logs to stdout
// - Subscriber 2: Sends traces to Tempo/Jaeger
// - Subscriber 3: Aggregates metrics (counters, histograms)
// - Subscriber 4: Generates flamegraphs for profiling
// - Subscriber 5: Your custom logic (alerts, business rules)
```

**The power:**
- **Multiple consumers:** Many subscribers process the same events
- **Runtime configuration:** Change subscribers without redeploying
- **Different purposes:** Same instrumentation → logs, metrics, traces, profiles
- **Structured data:** Subscribers get typed fields, not text to parse

**Mental model:** Tracing is a pub/sub system. You publish events, subscribers consume them however they want.

---

## The Problem with Traditional Logging

You deploy your code to production. At 3am, something breaks. You open the logs and see:

```
ERROR: Operation failed
ERROR: Connection timeout
WARN: Retry limit exceeded
```

**Questions you can't answer:**
- What operation failed?
- Which request caused this?
- What was the system doing before the error?
- Is this affecting all users or just one?
- Where did the timeout happen - database, network, external API?

**Why you can't answer them:**
- **No correlation:** Each log line is independent
- **No structure:** Errors are text, not typed data
- **No timing:** You don't know how long things took
- **No relationships:** Can't see parent-child operation hierarchies

### The Solution: Structured Events + Subscribers

**Tracing** means:
1. **Emit structured events** with typed fields (not text)
2. **Let subscribers consume them** for different purposes

Instead of hard-coding log output:
```
ERROR: Operation failed
```

You see:
```
ERROR: block validation failed
  request_id: a1b2c3d4
  component: asm_worker
  l1_height: 100
  l1_block: 347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30
  error_chain:
    1. signature verification failed
    2. public key mismatch
    3. expected: 02a1b2c3..., got: 03f4e5d6...
  duration: 45ms
  caller: process_block() at worker.rs:145
```

Now you can answer:
- **What failed:** Block validation (signature verification)
- **Which request:** a1b2c3d4
- **What block:** L1 block 100 (full ID provided)
- **Why:** Public key mismatch (with exact values)
- **Where:** asm_worker component, process_block function
- **How long:** 45ms

You can debug this without adding more logs and redeploying.

---

## The Subscriber Model: One Instrumentation, Many Consumers

This is the **key insight** that makes tracing different from logging.

### How Traditional Logging Works

```
┌──────────────┐
│  Your Code   │
└──────┬───────┘
       │ log::info!("...")
       ↓
┌──────────────┐
│   stdout     │  ← Single destination
└──────────────┘
```

You decide the format when you write the code. That's it.

### How Tracing with Subscribers Works

```
┌──────────────┐
│  Your Code   │  ← Emit structured events ONCE
└──────┬───────┘
       │ tracing::info_span!(...)
       │ tracing::info!(field1 = val1, field2 = val2, "message")
       ↓
┌──────────────────────────────────────────────┐
│         Tracing Subscriber Registry          │
└───┬────────┬──────────┬──────────┬──────────┘
    │        │          │          │
    ↓        ↓          ↓          ↓
┌────────┐ ┌─────┐ ┌─────────┐ ┌──────────┐
│ fmt    │ │OTLP │ │ metrics │ │  custom  │
│ (logs) │ │     │ │         │ │ business │
└────────┘ └─────┘ └─────────┘ └──────────┘
    │        │          │          │
    ↓        ↓          ↓          ↓
  stdout   Tempo    Prometheus   Alerts
```

**Multiple subscribers** process the same events, each doing something different.

---

### Real Example: What Each Subscriber Does

You write **one instrumentation**:

```rust
let span = info_span!(
    "process_block",
    component = "asm_worker",
    l1_height = 100,
    l1_block = "347e16b7...",
);
let _enter = span.enter();

info!(duration_ms = 45, "block validation complete");
```

**Subscriber 1: `fmt` (logs to stdout/files)**
```
2025-12-03T07:13:37.827Z INFO component=asm_worker l1_height=100 l1_block=347e16b7... duration_ms=45 block validation complete
```
→ Human-readable logs for debugging

**Subscriber 2: `opentelemetry` (sends traces to Tempo/Jaeger)**
```json
{
  "traceId": "a1b2c3d4",
  "spanId": "e5f6g7h8",
  "name": "process_block",
  "attributes": {
    "component": "asm_worker",
    "l1_height": 100,
    "l1_block": "347e16b7..."
  },
  "events": [{
    "name": "block validation complete",
    "attributes": {"duration_ms": 45}
  }],
  "duration": "45ms"
}
```
→ Distributed traces for cross-service debugging

**Subscriber 3: `metrics` (aggregates to Prometheus)**
```
block_processing_duration_seconds{component="asm_worker"} 0.045
block_processing_total{component="asm_worker"} 1547
```
→ Time-series metrics for dashboards and alerts

**Subscriber 4: `flamegraph` (generates performance profiles)**
```
process_block                       45ms ████████████████
  ├─ validate_block                 30ms ██████████
  └─ store_block                    15ms █████
```
→ Identify bottlenecks

**Subscriber 5: Custom business logic**
```rust
struct AlertSubscriber;

impl Subscriber for AlertSubscriber {
    fn on_event(&self, event: &Event) {
        if event.fields().get("error_type") == Some("ValidationFailed") {
            send_pagerduty_alert(event);
        }
    }
}
```
→ Alert on specific patterns

---

### The Key Insight

**You instrument your code ONCE.**

**Every subscriber extracts what it needs.**

This is fundamentally different from logging where you have to choose:
- "Do I log this for debugging or for metrics?"
- "Should this be INFO or DEBUG?"
- "Do I log the duration or skip it to reduce noise?"

With tracing + subscribers:
- **Emit everything** as structured events
- **Each subscriber filters** what it cares about
- **Configure at runtime** without changing code

---

## Why Subscribers Matter More Than Verbosity

### The Logging Mindset (Wrong)

"I need to add more logs to understand what's happening."

```rust
log::debug!("Starting block validation");
log::debug!("Validating signature...");
log::debug!("Signature valid");
log::debug!("Validating state transition...");
log::debug!("State transition valid");
log::info!("Block validation complete");
```

**Problems:**
- ❌ **Log spam:** 6 lines for one operation
- ❌ **Fixed verbosity:** Either you log all of this or none
- ❌ **No metrics:** You don't get timing/counters without separate code
- ❌ **No structure:** Just text

### The Tracing Mindset (Right)

"I need to emit structured events. Subscribers will extract what they need."

```rust
let span = info_span!(
    "validate_block",
    component = "asm_worker",
    l1_height = 100,
);
let _enter = span.enter();

let sig_span = debug_span!("validate_signature");
validate_signature().instrument(sig_span)?;

let state_span = debug_span!("validate_state_transition");
validate_state_transition().instrument(state_span)?;

info!("validation complete");
```

**Benefits:**
- ✅ **fmt subscriber (DEBUG level):** Shows all 3 spans → detailed debugging
- ✅ **fmt subscriber (INFO level):** Shows only outer span → clean logs
- ✅ **opentelemetry subscriber:** Records all 3 spans with timing
- ✅ **metrics subscriber:** Counts validations, records duration percentiles
- ✅ **No extra code:** Spans automatically record entry/exit and duration

**Same instrumentation, multiple uses:**
- Debugging: Enable DEBUG level, see everything
- Production: Enable INFO level, see only summaries
- Metrics: Always aggregating in background
- Traces: Always capturing for sampled requests

---

## Key Concepts: Spans, Events, and Subscribers

### 1. Subscribers: The Core Abstraction

A **subscriber** is a component that receives and processes tracing events.

**Think of subscribers as:**
- Event listeners in the tracing system
- Each subscriber has a different purpose
- Multiple subscribers can run simultaneously
- You configure them at startup or runtime

**Common subscribers:**
- **`fmt`:** Formats events as text logs (`tracing_subscriber::fmt`)
- **`opentelemetry`:** Sends spans/events to trace backends (`tracing_opentelemetry`)
- **`metrics`:** Aggregates span durations into histograms (`metrics_tracing_context`)
- **`tokio-console`:** Real-time async task monitoring
- **Custom:** Your business logic

**Example configuration:**

```rust
use tracing_subscriber::{layer::SubscriberExt, Registry};

let fmt_layer = tracing_subscriber::fmt::layer();
let otel_layer = tracing_opentelemetry::layer();
let metrics_layer = MetricsLayer::new();

tracing_subscriber::registry()
    .with(fmt_layer)      // Subscriber 1: logs
    .with(otel_layer)     // Subscriber 2: traces
    .with(metrics_layer)  // Subscriber 3: metrics
    .init();
```

Now every `info_span!()` and `info!()` call goes to all 3 subscribers!

---

### 2. Events

An **event** is a single point-in-time record of something that happened.

**Example:**
```rust
info!(user_id = 123, action = "login", "user logged in");
```

Events are like log lines, but with structured fields that subscribers can extract.

---

### 3. Spans

A **span** represents a unit of work with a start time and end time. Spans show **operations over time**.

**Think of a span as:**
- A function call that takes time to complete
- A database query
- An RPC request
- Processing a message

**Example Span:**
```
[━━━━━━━ process_block() ━━━━━━━]
start: 07:13:37.782
end:   07:13:37.827
duration: 45ms
```

**Spans can contain:**
- **Attributes:** Metadata about the operation (request ID, entity IDs)
- **Events:** Things that happened during the span
- **Status:** Did it succeed or fail?

**Rust Code:**
```rust
let span = info_span!(
    "process_block",
    component = "asm_worker",
    l1_height = 100,
);

async move {
    // Work happens here
    validate_block(&block)?;
    store_block(&block)?;
}
.instrument(span)  // Attaches the span to this work
.await
```

### 3. Traces

A **trace** is a collection of spans that shows the complete journey of a request through your system.

**Think of a trace as:**
- A request's entire lifecycle from entry to exit
- The call stack + timing information
- A story told through connected operations

**Visual Example:**

```
Trace ID: a1b2c3d4

┌─────────────────────────────────────────────────┐
│  Handle L1 Block 100                    (500ms) │
│  ┌─────────────────────────────────────┐        │
│  │  ASM Worker: Process Block   (100ms)│        │
│  │  ┌─────────────────┐                │        │
│  │  │ Validate (45ms) │                │        │
│  │  └─────────────────┘                │        │
│  │  ┌─────────────────────┐            │        │
│  │  │ Store to DB (50ms)  │            │        │
│  │  └─────────────────────┘            │        │
│  └─────────────────────────────────────┘        │
│  ┌──────────────────────────────────────┐       │
│  │  Fork Choice: Update Tip    (200ms)  │       │
│  │  ┌──────────────────┐                │       │
│  │  │ DB Query (150ms) │                │       │
│  │  └──────────────────┘                │       │
│  └──────────────────────────────────────┘       │
│  ┌──────────────────────────────────────┐       │
│  │  Notify Clients             (100ms)  │       │
│  │  ┌──────────────┐                    │       │
│  │  │ RPC (80ms)   │                    │       │
│  │  └──────────────┘                    │       │
│  └──────────────────────────────────────┘       │
└─────────────────────────────────────────────────┘
```

Each box is a **span**. The hierarchy shows **parent-child relationships**. The entire collection is a **trace**.

**Key Point:** All these spans share the same **trace ID** (a1b2c3d4), so you can filter logs to see only this request.

### 4. Context Propagation

**The problem:** When operation A calls operation B, how does B know it's part of the same trace?

**The solution:** Pass a **trace context** that contains:
- `trace_id`: The unique ID for this entire operation
- `span_id`: The ID of the current span
- `parent_span_id`: The ID of the calling span (if any)

**Example:**

```rust
// Service A creates a trace context
let trace_ctx = TraceContext::new_root();  // trace_id = a1b2c3d4

// Service A calls Service B via RPC
rpc_client.call_with_trace("method", params, trace_ctx).await?;

// Service B receives the trace context and continues the trace
fn handle_rpc(trace_ctx: TraceContext) {
    let span = info_span!(
        "rpc_handler",
        req_id = %trace_ctx.trace_id(),  // Still a1b2c3d4!
    );
    // Now all logs in Service B have the same request ID
}
```

**This is called "distributed tracing"** because the trace spans multiple services.

### 5. Structured Logging

**Traditional logging (unstructured):**
```rust
info!("Processing block 100 for user admin");
```

Output: `Processing block 100 for user admin`

**Problem:** You can't filter or query this. It's just text.

**Structured logging:**
```rust
info!(
    l1_height = 100,
    user = "admin",
    component = "asm_worker",
    "processing block"
);
```

Output: `processing block l1_height=100 user=admin component=asm_worker`

**Now you can:**
```bash
$ grep "l1_height=100" logs.txt    # Find all logs about block 100
$ grep "component=asm_worker" logs.txt  # Find all ASM worker logs
```

Or in a log aggregation system:
```
query: {component="asm_worker"} | l1_height > 100
```

---

## The Three Pillars

Observability has three complementary approaches:

### 1. Logs

**What:** Timestamped text records of discrete events

**Best for:**
- Debugging specific errors
- Seeing exact values at failure points
- Audit trails

**Example:**
```
2025-12-03T07:13:37.782Z ERROR: signature verification failed expected=02a1b2c3 got=03f4e5d6
```

### 2. Metrics

**What:** Numerical measurements aggregated over time

**Best for:**
- Dashboards and alerting
- Seeing trends (requests/sec, error rate, latency percentiles)
- Resource usage (CPU, memory, disk)

**Example:**
```
block_processing_duration_ms{component="asm_worker"} p50=45 p99=120
blocks_processed_total{component="asm_worker"} 1547
```

### 3. Traces

**What:** Detailed request lifecycle with timing and relationships

**Best for:**
- Understanding cross-service flows
- Identifying performance bottlenecks
- Seeing the complete picture of an operation

**Example:**
```
Trace a1b2c3d4:
  ├─ handle_l1_block (500ms)
  │  ├─ asm_worker.process (100ms)
  │  ├─ fork_choice.update (200ms) ← Bottleneck!
  │  └─ notify_clients (100ms)
```

**They work together:**
- **Metrics** tell you *something is wrong* (error rate spike)
- **Traces** tell you *where* the problem is (which service, which operation)
- **Logs** tell you *why* it failed (exact error message and context)

---

## Distributed Tracing Explained

### Why Distributed Systems Are Hard to Debug

**Single-Process System:**
```
User Request
  ↓
Function A
  ↓
Function B
  ↓
Function C
  ↓
Response

Call stack shows you the path!
```

**Distributed System:**
```
User Request
  ↓
Service 1 → RPC → Service 2
  ↓                  ↓
DB Query          RPC → Service 3
                        ↓
                    External API
                        ↓
                  ← Response ←
  ↓
Response

Call stack is broken! Each service only sees its part.
```

**The problem:** Service 2 doesn't know it's part of the same operation as Service 1. Logs from different services are completely disconnected.

### How Distributed Tracing Solves This

**Step 1: Generate a Trace ID**
```rust
// In Service 1:
let trace_id = Uuid::new_v4();  // a1b2c3d4
```

**Step 2: Attach it to all logs**
```rust
info!(req_id = %trace_id, "processing request");
```

**Step 3: Propagate it across RPC**
```rust
// Service 1 calls Service 2:
let headers = vec![("traceparent", format!("00-{}-...", trace_id))];
rpc_client.call_with_headers("method", params, headers).await?;
```

**Step 4: Extract it on the other side**
```rust
// Service 2 receives the call:
fn handle_rpc(headers: Headers) {
    let trace_id = extract_trace_id(&headers);  // Still a1b2c3d4!
    info!(req_id = %trace_id, "handling RPC");
}
```

**Result:**
```bash
$ grep "req_id=a1b2c3d4" service1.log service2.log service3.log

service1.log: req_id=a1b2c3d4 component=api_handler processing request
service1.log: req_id=a1b2c3d4 component=api_handler calling service2
service2.log: req_id=a1b2c3d4 component=rpc_handler received request
service2.log: req_id=a1b2c3d4 component=worker querying database
service3.log: req_id=a1b2c3d4 component=external_api calling API
service3.log: req_id=a1b2c3d4 component=external_api API call failed error="timeout"
service2.log: req_id=a1b2c3d4 component=rpc_handler request failed
service1.log: req_id=a1b2c3d4 component=api_handler returning error
```

You can now see the **entire flow** across all services by grepping for one ID!

### The Standard: OpenTelemetry

**OpenTelemetry** is the industry standard for distributed tracing. It provides:

1. **Semantic conventions:** Standard names for common attributes (like `http.method`, `db.system`)
2. **Context propagation:** Standard way to pass trace context (`traceparent` header format)
3. **SDKs:** Libraries for Rust, Go, Python, etc. to instrument your code
4. **Exporters:** Send traces to Jaeger, Tempo, Datadog, etc.

**You don't reinvent this.** You use OpenTelemetry and follow the standard.

---

## Why This Matters for Vertex Core

### Our Architecture

Vertex Core is a **distributed blockchain system** with:

- **Multiple services:** Sequencer, full node, L1 reader, EVM executor
- **Actor model:** Async tasks processing messages concurrently
- **RPC boundaries:** Services communicate via jsonrpsee
- **High throughput:** Processing hundreds of blocks per second
- **Financial transactions:** Bugs cost real money

### Current Problems (From Production Logs)

**Problem 1: Can't follow a request**
```
07:13:37.782  INFO  ASM found pivot anchor state pivot_block=100
07:13:37.801  INFO  Successfully reassigned expired assignments
07:13:44.958  WARN  tripped bail interrupt, exiting
```

**Question:** Which operation triggered the crash?
**Answer:** Unknown. Logs are interleaved, no correlation.

**Problem 2: Can't filter by component**
```
07:13:37.954  INFO  processing new block slot=1
07:13:37.955  INFO  processing new block slot=2
```

**Question:** Are these from fork choice manager or ASM worker?
**Answer:** Module path gives a clue, but not consistent across codebase.

**Problem 3: Can't trace across RPC**
```
Client: RPC call to getBlockTemplate failed
Server: All requests succeeded
```

**Question:** Are they talking about the same request?
**Answer:** Unknown. No shared ID.

### What Good Observability Looks Like for Us

**Scenario: Production Crash**

**With current logging:**
1. Service crashes at 3am
2. Look at 100,000 lines of jumbled logs
3. Spend 2 hours trying to reconstruct what happened
4. Give up, restart service, pray it doesn't happen again

**With proper observability:**
1. Service crashes at 3am
2. Alert includes: `trace_id=a1b2c3d4`
3. Run: `grep "req_id=a1b2c3d4" service.log`
4. See complete timeline:
   ```
   07:13:44.950  INFO  component=sequencer req_id=a1b2c3d4 duty_type=sign_block
   07:13:44.951  INFO  component=rpc_client req_id=a1b2c3d4 calling getBlockTemplate
   07:13:44.956  ERROR component=rpc_server req_id=a1b2c3d4 getBlockTemplate timeout
   07:13:44.957  ERROR component=sequencer req_id=a1b2c3d4 triggering bail reason="RPC timeout"
   07:13:44.958  WARN  component=bail_manager req_id=a1b2c3d4 BAIL TRIGGERED
   ```
5. Root cause identified in 5 minutes: RPC timeout
6. Fix: increase timeout, add retries, deploy

**Observability makes debugging systematic instead of guesswork.**

---

## Key Terminology Reference

| Term | Definition | Example |
|------|------------|---------|
| **Subscriber** | Component that processes tracing events | `fmt` subscriber logs to stdout |
| **Layer** | Composable piece of subscriber functionality | `OpenTelemetryLayer`, `FmtLayer` |
| **Registry** | Manages multiple subscribers | `tracing_subscriber::registry()` |
| **Event** | A single point-in-time log entry | `info!(field = value, "message")` |
| **Span** | A unit of work with duration | `info_span!("process_block")` |
| **Trace** | Collection of related spans | All operations for request a1b2c3d4 |
| **Trace ID** | Unique ID for entire trace | `a1b2c3d4-e5f6-7g8h-9i0j-k1l2m3n4o5p6` |
| **Span ID** | Unique ID for a single span | `b2c3d4e5-f6g7-h8i9-j0k1-l2m3n4o5p6q7` |
| **Parent Span** | The span that called this span | If A calls B, A is parent of B |
| **Context** | Metadata passed between operations | Contains trace_id, span_id, baggage |
| **Baggage** | User-defined key-value data in context | `user_id=123`, `tenant=acme` |
| **Field** | Key-value metadata on spans/events | `component=asm_worker`, `l1_height=100` |
| **Instrumentation** | Emitting structured events from code | Creating spans, emitting events |
| **Component** | Semantic tag for subsystem | `fork_choice_manager`, `asm_worker` |
| **Request ID** | Synonym for trace_id | `req_id=a1b2c3d4` |
| **Correlation** | Linking related events via ID | All events with `req_id=a1b2c3d4` |
| **Context Propagation** | Passing trace context across boundaries | Sending trace_id in RPC headers |
| **Exporter** | Backend-specific output from subscriber | OTLP exporter to Tempo |
| **Sampler** | Decides which traces to record | Sample 1% of traces to reduce overhead |
| **Filter** | Controls which events subscribers see | `info` level shows INFO+ events |

---

## Common Subscribers and Their Uses

### `fmt` Subscriber - Human-Readable Logs

**Purpose:** Format events as text for humans to read

**Output:** stdout, stderr, or files

**Use cases:**
- Development debugging
- Production log files
- grep-ing through logs

**Configuration:**
```rust
tracing_subscriber::fmt()
    .with_max_level(Level::INFO)  // Filter level
    .with_target(false)            // Hide module paths
    .compact()                     // Compact format
    .init();
```

---

### `opentelemetry` Subscriber - Distributed Traces

**Purpose:** Send spans to trace backends (Tempo, Jaeger, Datadog)

**Output:** OTLP protocol to trace collector

**Use cases:**
- Cross-service request tracing
- Performance analysis (flame graphs)
- Understanding call hierarchies

**Configuration:**
```rust
use tracing_opentelemetry::OpenTelemetryLayer;

let tracer = opentelemetry_otlp::new_pipeline()
    .tracing()
    .with_exporter(...)
    .install_batch()?;

let otel_layer = OpenTelemetryLayer::new(tracer);

tracing_subscriber::registry()
    .with(otel_layer)
    .init();
```

---

### `metrics` Subscriber - Automatic Metrics from Spans

**Purpose:** Aggregate span durations into metrics (histograms, counters)

**Output:** Prometheus, StatsD, or other metrics systems

**Use cases:**
- Automatic request duration histograms
- Request count by component
- Error rates by operation type

**How it works:**
```rust
// You create a span:
let span = info_span!("process_block", component = "asm_worker");

// Metrics subscriber automatically creates:
// - Counter: block_processing_total{component="asm_worker"}
// - Histogram: block_processing_duration_seconds{component="asm_worker"}
```

**No manual metrics code needed!**

---

### `tokio-console` Subscriber - Async Task Debugging

**Purpose:** Real-time monitoring of async tasks

**Output:** Interactive TUI dashboard

**Use cases:**
- Finding stuck tasks
- Identifying task spawn overhead
- Visualizing async task trees

**Configuration:**
```rust
console_subscriber::init();
```

Then run `tokio-console` CLI to connect and see live task stats.

---

### Custom Subscribers - Business Logic

**Purpose:** Implement your own event processing

**Examples:**
- Alert on specific error patterns
- Log to multiple destinations (stdout + file + network)
- Collect custom metrics
- Implement audit logging
- Rate limit or sample events

**Example:**
```rust
struct PagerDutySubscriber;

impl<S: Subscriber> Layer<S> for PagerDutySubscriber {
    fn on_event(&self, event: &Event, _ctx: Context<S>) {
        if let Some(error_type) = event.fields().get("error_type") {
            if error_type == "CriticalValidationFailure" {
                send_pagerduty_alert(event);
            }
        }
    }
}
```

---

## The Three Pillars (Revisited with Subscribers)

### 1. Logs (via `fmt` subscriber)

**What:** Timestamped text records of discrete events

**Subscriber:** `tracing_subscriber::fmt`

**Best for:**
- Debugging specific errors
- Seeing exact values at failure points
- Audit trails

---

### 2. Metrics (via `metrics` subscriber)

**What:** Numerical measurements aggregated over time

**Subscriber:** `metrics_tracing_context` or custom

**Best for:**
- Dashboards and alerting
- Seeing trends (requests/sec, error rate, latency percentiles)
- Resource usage (CPU, memory, disk)

**Key insight:** Same spans that produce logs also produce metrics automatically!

---

### 3. Traces (via `opentelemetry` subscriber)

**What:** Detailed request lifecycle with timing and relationships

**Subscriber:** `tracing_opentelemetry`

**Best for:**
- Understanding cross-service flows
- Identifying performance bottlenecks
- Seeing the complete picture of an operation

---

## The Power of Composition

**Traditional logging:**
```rust
// Separate code for each purpose
log::info!("Request processed");                    // Logs
metrics::counter!("requests_total").increment(1);   // Metrics
tracer.span().record("request");                    // Traces
```

**Tracing with subscribers:**
```rust
// One instrumentation, multiple outputs
let span = info_span!("process_request", user_id = 123);
let _enter = span.enter();

// This SINGLE span produces:
// - Log line (fmt subscriber)
// - Trace span (opentelemetry subscriber)
// - Metric increment (metrics subscriber)
// - Custom alert check (your subscriber)
```

**No duplication. Compose subscribers as needed.**

---

## Common Questions

### "Isn't this just more verbose logging?"

**No. This is fundamentally different.**

**Verbose logging:**
```rust
log::debug!("Starting validation");
log::debug!("Checking signature");
log::debug!("Signature OK");
log::debug!("Checking state");
log::debug!("State OK");
log::info!("Validation complete");
```
- ❌ Fixed output (always goes to stdout/files)
- ❌ Single purpose (human-readable text)
- ❌ No structured data for machines
- ❌ Can't selectively consume

**Tracing with subscribers:**
```rust
let span = info_span!("validate_block");
let _enter = span.enter();

let sig_span = debug_span!("check_signature");
check_signature().instrument(sig_span)?;

let state_span = debug_span!("check_state");
check_state().instrument(state_span)?;
```
- ✅ Multiple outputs (logs, traces, metrics, custom)
- ✅ Multiple purposes (debugging, performance, monitoring)
- ✅ Structured data (subscribers extract fields)
- ✅ Each subscriber chooses what to consume

**The key difference:** Subscribers can extract different information from the same instrumentation.

---

### "Can I just use the `fmt` subscriber for logs?"

**Yes, but you're leaving value on the table.**

If you only use `fmt`:
```rust
tracing_subscriber::fmt().init();
```

You get:
- ✅ Structured logs
- ✅ Span context inheritance
- ✅ Automatic duration tracking

**But you're missing:**
- ❌ Distributed tracing (no Tempo/Jaeger visualizations)
- ❌ Automatic metrics (no histograms/counters)
- ❌ Async task profiling (no tokio-console)

**The beauty:** You can add more subscribers later without changing your code!

```rust
// Day 1: Just logs
tracing_subscriber::fmt().init();

// Day 30: Add tracing (no code changes!)
tracing_subscriber::registry()
    .with(fmt::layer())
    .with(opentelemetry_layer())
    .init();

// Day 60: Add metrics (no code changes!)
tracing_subscriber::registry()
    .with(fmt::layer())
    .with(opentelemetry_layer())
    .with(metrics_layer())
    .init();
```

---

### "We already log errors. Isn't that enough?"

**No. Logging tells you *what* broke. Tracing with subscribers tells you:**
- **Why** it broke (error chain)
- **When** it broke (timeline)
- **Where** it broke (exact component, function, line)
- **What preceded** it (events leading up to failure)
- **Who triggered** it (which user, which request)
- **How common** it is (is this the first time or the millionth?)

**Example:**
```
❌ Just error logging:
ERROR: validation failed

✅ With observability:
ERROR: block validation failed
  req_id: a1b2c3d4
  component: asm_worker
  l1_height: 100
  l1_block: 347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30
  error_chain:
    1. signature verification failed
    2. public key mismatch
    3. expected 02a1b2c3, got 03f4e5d6
  caller: process_block() at worker.rs:145
  duration: 45ms
  preceding_events:
    - L1 block 100 received at 07:13:37.780
    - ASM state transition started at 07:13:37.782
    - Signature check failed at 07:13:37.827
```

The second one is debuggable. The first one is not.

### "Won't this slow down my code?"

**Industry data:** Distributed tracing adds 1-3% CPU overhead and 2-3x log volume.

**Trade-off:** Would you pay 3% CPU to cut debugging time by 10x?

**Mitigation:**
- **Sampling:** Only record 10% of traces in high-traffic paths
- **Rate limiting:** Log hot loops once per second, not per iteration
- **Async recording:** Span recording happens in background, not on critical path
- **Smart filtering:** Only send errors to centralized logging, keep successes local

**Reality:** The cost of NOT having observability (hours of debugging, production incidents) far exceeds the cost of instrumentation.

### "This seems like a lot of work."

**Up-front cost:** Yes, adding spans and structured logging takes time.

**Ongoing benefit:** Every future bug is 10x faster to debug.

**Investment analogy:**
- **Without observability:** Spend 0 hours instrumenting, 100 hours debugging
- **With observability:** Spend 10 hours instrumenting, 10 hours debugging

You're trading up-front work for long-term efficiency.

---

## Reading Order

Now that you understand the concepts, here's how to proceed:

### 1. **Start here:** `getting-started.md` (this document)
**Purpose:** Learn what observability is and why it matters

### 2. **Read next:** `improvements-in-words.md`
**Purpose:** Understand the specific problems in Vertex Core and how observability solves them

### 3. **Then read:** `standards-to-follow.md`
**Purpose:** Learn the mandatory engineering standards for instrumentation

### 4. **Finally read:** `improvements.md`
**Purpose:** See complete code examples and implementation details

### 5. **Start instrumenting:**
- Pick one small component (e.g., ASM worker)
- Add spans with required fields (`component`, `req_id`)
- Add structured error logging
- Test: Can you grep for a request and see its full lifecycle?
- Iterate: Expand to other components

---

## Core Principles (Simplified)

After reading this guide, remember these principles:

### Principle 1: Every operation gets a unique ID
**Why:** So you can find all related events

### Principle 2: Every log has context
**Why:** So you know what it's about (which block, which component, which request)

### Principle 3: IDs flow through the entire system
**Why:** So cross-service operations stay connected

### Principle 4: Use structured fields, not just messages
**Why:** So logs are machine-queryable, not just human-readable

### Principle 5: Summarize hot paths, don't spam
**Why:** So logs stay readable and storage doesn't explode

---

## Next Steps

1. **Read** `improvements-in-words.md` to see how these concepts apply to Vertex Core
2. **Review** production logs with a critical eye: Can you follow a request? Can you filter by component?
3. **Ask questions** if anything is unclear - observability is a skill that takes time to develop
4. **Start small** - instrument one component, measure the benefit, expand

**Remember:** Observability is not optional for distributed systems. It's the difference between systematic debugging and chaos.

---

## Further Learning

### Books
- **"Distributed Tracing in Practice"** by Austin Parker et al. (O'Reilly, 2020)
- **"Observability Engineering"** by Charity Majors et al. (O'Reilly, 2022)

### Online Resources
- **OpenTelemetry Documentation:** https://opentelemetry.io/docs/
- **Rust tracing crate:** https://docs.rs/tracing/
- **Google Dapper Paper (2010):** https://research.google/pubs/pub36356/
- **W3C Trace Context Spec:** https://www.w3.org/TR/trace-context/

### Practice
- Instrument a small personal project with tracing
- Read through well-instrumented open source projects
- Use Jaeger or Tempo locally to visualize traces

**The best way to learn tracing is by practicing it.** Start adding spans to your code today.

---

## Core Principles (Simplified)

After reading this guide, remember these principles:

### Principle 1: Emit structured events, not log strings
**Traditional:** `log::info!("Processing block 100")`
**Tracing:** `info!(l1_height = 100, "processing block")`

### Principle 2: Use spans for operations, not just events
**Traditional:** `log::info!("Starting validation"); validate(); log::info!("Validation complete");`
**Tracing:** `let span = info_span!("validate"); let _enter = span.enter(); validate();`

### Principle 3: Let subscribers decide what to do with events
**Traditional:** Hard-code log format in every log call
**Tracing:** Configure subscribers at startup, instrument once

### Principle 4: Compose multiple subscribers for multiple purposes
**Traditional:** Separate code for logs, metrics, traces
**Tracing:** One instrumentation → multiple subscribers extract what they need

### Principle 5: Context flows automatically via spans
**Traditional:** Manually pass request IDs to every function
**Tracing:** Set fields on span, all events inherit automatically

---

## Final Summary: The Mental Shift

### From Logging to Tracing

**Old mental model (logging):**
> "I need to add log statements to see what's happening. Each log goes to stdout."

**New mental model (tracing):**
> "I need to emit structured events. Subscribers will extract logs, metrics, traces, and whatever else they need."

### The Key Insight

**Logging is hard-coded output.**
You decide the format and destination when you write the code.

**Tracing is event emission + pluggable consumers.**
You emit events once. Subscribers process them in different ways.

### What You Gain

1. **Flexibility:** Add new subscribers without changing code
2. **Efficiency:** One instrumentation, many outputs
3. **Power:** Same events → logs, metrics, traces, profiles, alerts
4. **Structure:** Machines can query/aggregate, not just humans reading text
5. **Timing:** Automatic duration tracking on all spans

### What You Must Understand

- **Spans** are the units of work (with duration)
- **Events** are point-in-time records (like log lines)
- **Subscribers** consume spans/events for different purposes
- **Fields** are structured data (not text to parse)
- **Context** flows automatically via span inheritance

### The Tracing Checklist

When adding instrumentation, ask:

1. [ ] Am I creating a span for this operation?
2. [ ] Does my span have `component` and `req_id` fields?
3. [ ] Are my events using structured fields, not string formatting?
4. [ ] Am I using `.instrument()` or `.enter()` to attach the span?
5. [ ] Have I configured appropriate subscribers for my needs?

---

## Why Subscribers Are The Killer Feature

**Traditional approach:**
```rust
// Need logs?
log::info!("Request processed");

// Need metrics?
metrics::counter!("requests_total").increment(1);

// Need traces?
tracer.create_span("process_request");

// Three separate pieces of code, three separate systems
```

**Tracing approach:**
```rust
// One instrumentation:
let span = info_span!("process_request");
let _enter = span.enter();

// Gets you:
// - Logs (fmt subscriber)
// - Metrics (metrics subscriber)
// - Traces (opentelemetry subscriber)
// - Whatever else you configure
```

**The breakthrough:** Decoupling instrumentation from consumption.

You write instrumentation once. Subscribers extract value in different ways. Add new subscribers later without touching your code.

**This is why tracing > logging.**
