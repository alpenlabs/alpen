# Logging & Tracing Standards for Vertex Core

**Version:** 1.0
**Last Updated:** 2025-12-03
**Status:** Engineering Standard - MANDATORY

## Table of Contents

1. [Quick Start: The Minimal Pattern](#quick-start-the-minimal-pattern) ⭐ **Start here**
2. [References & Standards](#references--standards)
3. [Core Principles](#core-principles)
4. [Mandatory Field Standards](#mandatory-field-standards)
5. [Span Creation Guidelines](#span-creation-guidelines)
6. [Error Handling & Logging](#error-handling--logging)
7. [RPC Instrumentation](#rpc-instrumentation)
8. [Performance Considerations](#performance-considerations)
9. [Real Examples From Production Logs](#real-examples-from-production-logs) ⭐ **See concrete problems**
10. [Anti-Patterns & Code Smells](#anti-patterns--code-smells)
11. [Clippy Rules & Enforcement](#clippy-rules--enforcement)
12. [Testing Requirements](#testing-requirements)
13. [Code Review Checklist](#code-review-checklist)

---

## References & Standards

This document synthesizes best practices from the following industry standards and resources:

### Primary Standards

1. **OpenTelemetry Specification**
   - Source: https://opentelemetry.io/docs/specs/otel/
   - What we use: Semantic conventions for traces, spans, and attributes
   - Specifically: https://opentelemetry.io/docs/specs/semconv/general/trace/

2. **W3C Trace Context**
   - Source: https://www.w3.org/TR/trace-context/
   - What we use: Standard for propagating trace context across service boundaries (traceparent/tracestate headers)

3. **Rust Tracing Crate Documentation**
   - Source: https://docs.rs/tracing/
   - What we use: Structured, composable tracing for Rust applications

### Industry Best Practices

4. **Google SRE Book - Chapter 20: Load Balancing in the Datacenter**
   - Source: https://sre.google/sre-book/load-balancing-datacenter/
   - Principle: Request IDs for correlation across distributed systems

5. **Google Dapper Paper (2010)**
   - Source: https://research.google/pubs/pub36356/
   - Seminal paper on distributed tracing at scale
   - Principle: Span-based tracing with parent-child relationships

6. **Uber Jaeger Documentation**
   - Source: https://www.jaegertracing.io/docs/
   - What we use: Best practices for trace instrumentation and sampling

7. **The Twelve-Factor App - XI. Logs**
   - Source: https://12factor.net/logs
   - Principle: "Treat logs as event streams"

### Observability Resources

8. **"Distributed Tracing in Practice" by Austin Parker et al.**
   - Publisher: O'Reilly, 2020
   - Concepts: Trace context propagation, span design patterns

9. **Charity Majors (Honeycomb.io) - Observability Engineering**
   - Blog: https://www.honeycomb.io/blog
   - Concepts: High-cardinality logging, structured events over logs

10. **Cloud Native Computing Foundation (CNCF) Observability Whitepaper**
    - Source: https://github.com/cncf/tag-observability/blob/main/whitepaper.md
    - Concepts: Three pillars of observability (logs, metrics, traces)

### Performance & Overhead Studies

11. **"Performance Analysis of Distributed Tracing Overhead" (ACM SIGOPS)**
    - Finding: 1-5% overhead for production tracing systems
    - Our target: 1-3% based on this research

---

## Quick Start: The Minimal Pattern

**TL;DR for busy developers:**

```rust
// Step 1: Create a span with context (do this ONCE)
let span = info_span!(
    "operation_name",
    component = "your_component",
    req_id = %trace_ctx.short_id(),
    // Add entity IDs (l1_height, l2_slot, etc.)
);

// Step 2: Wrap your work
async move {
    // Step 3: Log without repeating context - it's inherited!
    info!("operation started");
    do_work()?;
    info!("operation completed");
}
.instrument(span)
.await
```

**That's it.** All logs inside automatically get `component`, `req_id`, and entity fields. No manual duplication.

**Timing is free:** When the span closes, you automatically get a log with duration.

**Read below for details and rules.**

---

## Core Principles

### Proposed Telemetry Principles for Vertex Core

These principles are synthesized from the standards above, adapted to our actor-based async architecture:

### **Principle 0: Attach Context Once at Span Level (MOST IMPORTANT)**

**The Lazy Developer's Rule:** Set context ONCE on the span, not on every log line.

```rust
// ✅ CORRECT: Set context once on span
let span = info_span!(
    "process_block",
    component = "asm_worker",      // ← Set once
    req_id = %trace_ctx.short_id(), // ← Set once
    l1_height = 100,                // ← Set once
);

async move {
    // All these logs automatically inherit component, req_id, l1_height!
    info!("starting validation");           // ← No manual fields needed
    validate_block(&block)?;
    info!("validation complete");           // ← No manual fields needed
    store_block(&block)?;
    info!("block stored");                  // ← No manual fields needed
}
.instrument(span)
.await
```

**Output (automatic field inheritance):**
```
component=asm_worker req_id=a1b2c3d4 l1_height=100 starting validation
component=asm_worker req_id=a1b2c3d4 l1_height=100 validation complete
component=asm_worker req_id=a1b2c3d4 l1_height=100 block stored
```

**Why This Matters:**
- ✅ **Less typing:** Set context once, not N times
- ✅ **Can't forget:** Context is inherited automatically
- ✅ **Easier refactoring:** Change span fields in one place
- ✅ **Free timing:** Span automatically records duration

**When to Use Spans vs Manual Fields:**
- **Use spans:** 99% of the time (any logical operation with multiple log points)
- **Manual fields:** Only for one-off logs outside any operation context

---

### Principle 1: Every logical operation MUST have a request ID
   - **Based on:** OpenTelemetry trace_id, Google Dapper paper, W3C Trace Context
   - **Why:** In distributed async systems, timestamp ordering is insufficient for correlation
   - **How:** Attach `req_id` to the span, not individual logs

### Principle 2: Every span MUST have a `component` field
   - **Based on:** OpenTelemetry semantic conventions (service.name, component attributes)
   - **Why:** Semantic filtering requires consistent tagging independent of code structure
   - **How:** Attach `component` to the span, not individual logs

### Principle 3: Entity IDs MUST be full, never abbreviated
   - **Why:** Abbreviated IDs break grep/filtering, making logs non-queryable
   - **How:** Use `l1_block = %full_id` in span fields

### Principle 4: High-frequency logs MUST be rate-limited
   - **Based on:** Production observability practices (Uber Jaeger sampling, Honeycomb head-based sampling)
   - **Why:** Unbounded log volume degrades performance and storage; intelligent sampling preserves signal
   - **How:** Use spans with sampling, or explicit rate limiting macros

### When to Log

```rust
// ✅ DO: Log at operation boundaries
info!("processing L1 block");     // Start of operation
info!("L1 block processed");      // End of operation

// ✅ DO: Log state transitions
info!(old_state = "pending", new_state = "finalized", "state transition");

// ✅ DO: Log errors with full context
error!(error = %e, l1_height = 100, "failed to process block");

// ❌ DON'T: Log in tight loops without rate limiting
for item in items {
    debug!("processing item");  // BAD: Spams logs
}

// ❌ DON'T: Log redundant information
info!("starting to process");
info!("processing");            // BAD: Adds no value
info!("finished processing");

// ❌ DON'T: Log sensitive information
debug!(private_key = %key);     // BAD: Security risk
```

---

## Mandatory Field Standards

### Required Fields for All Spans

Every span MUST include these fields:

```rust
// MANDATORY fields:
component = "service_name"        // Which subsystem (fork_choice_manager, asm_worker, etc.)
req_id = %trace_ctx.short_id()   // Request correlation ID

// REQUIRED for operation spans:
// Include the primary entity being operated on
l1_height = 100                   // For L1 operations
l1_block = %blkid                 // Full block ID
l2_slot = 8                       // For L2 operations
l2_block = %blkid                 // Full block ID
epoch = 1                         // For epoch operations
```

### Field Naming Convention

| Entity Type | Height/Slot Field | ID Field | Example |
|-------------|------------------|----------|---------|
| L1 Block | `l1_height` | `l1_block` | `l1_height = 100, l1_block = "347e16b7..."` |
| L2 Block | `l2_slot` | `l2_block` | `l2_slot = 8, l2_block = "90683b47..."` |
| Epoch | `epoch` | N/A | `epoch = 1` |
| Transaction | `tx_index` | `tx_hash` | `tx_index = 5, tx_hash = "abc123..."` |
| RPC Call | N/A | `rpc_method` | `rpc_method = "getBlockTemplate"` |

### ✅ CORRECT Field Usage

```rust
info_span!(
    "process_l1_block",
    component = "asm_worker",           // ✅ Identifies subsystem
    req_id = %trace_ctx.short_id(),    // ✅ Correlation ID
    l1_height = 100,                    // ✅ Consistent naming
    l1_block = %block.blkid(),         // ✅ Full ID, not abbreviated
)
```

### ❌ INCORRECT Field Usage

```rust
// BAD: Inconsistent naming
info_span!(
    "process_block",
    blkid = "aa026e..91422d",          // ❌ Abbreviated ID
    height = 100,                       // ❌ Ambiguous: L1 or L2?
    block_commitment = %blkid,          // ❌ Inconsistent with convention
)

// BAD: Missing required fields
info_span!(
    "process_l1_block",
    // ❌ Missing component field
    // ❌ Missing req_id field
    l1_height = 100,
)
```

---

## Span Creation Guidelines

### THE GOLDEN RULE: Spans Over Manual Fields

**Before you write `info!(component = "...", req_id = %..., ...)`**

**Ask yourself:** "Am I inside an operation that could have a span?"

- ✅ **YES:** Create a span with those fields, let logs inherit them
- ❌ **NO:** Only then use manual fields on individual log lines

**Examples of "operations that should have spans":**
- Processing a message/event
- Handling an RPC request
- A batch operation (processing N blocks)
- Any function that takes >1ms and has multiple log points
- Any operation you might want to measure timing for

**Example of where manual fields are OK:**
- One-off startup logs (`INFO: server started on port 8080`)
- Immediate error returns with no other logging

---

### Rule 1: One Span Per Logical Operation

```rust
// ✅ CORRECT: Single span for entire operation
async fn process_block(block: L1Block, ctx: TraceContext) -> Result<()> {
    let span = info_span!(
        "process_block",
        component = "asm_worker",        // ← All logs inherit this
        req_id = %ctx.short_id(),         // ← All logs inherit this
        l1_height = block.height,         // ← All logs inherit this
        l1_block = %block.id,             // ← All logs inherit this
    );

    async move {
        // No manual fields needed - all inherited from span!
        info!("starting validation");
        validate_block(&block)?;
        info!("validation complete");

        store_block(&block)?;
        info!("block stored");

        notify_processed(&block)?;
        info!("notifications sent");

        Ok(())
    }
    .instrument(span)
    .await
}

// ❌ INCORRECT: Multiple spans for single operation
async fn process_block(block: L1Block) -> Result<()> {
    let span1 = info_span!("validate");
    validate_block(&block).instrument(span1).await?;

    let span2 = info_span!("store");
    store_block(&block).instrument(span2).await?;

    // BAD: Loses parent-child relationship, harder to trace
}
```

### Rule 2: Use Sub-Spans for Distinct Sub-Operations

```rust
// ✅ CORRECT: Parent span with child spans for distinct phases
async fn process_block(block: L1Block, ctx: TraceContext) -> Result<()> {
    let span = info_span!(
        "process_block",
        component = "asm_worker",
        req_id = %ctx.short_id(),
        l1_block = %block.id,
    );

    async move {
        // Child span for validation
        let validation_span = debug_span!(
            "validate_block",
            component = "asm_worker",
            req_id = %ctx.short_id(),
        );
        validate_block(&block).instrument(validation_span).await?;

        // Child span for storage
        let storage_span = debug_span!(
            "store_block",
            component = "asm_worker",
            req_id = %ctx.short_id(),
        );
        store_block(&block).instrument(storage_span).await?;

        Ok(())
    }
    .instrument(span)
    .await
}
```

### Rule 3: Propagate Context Across Boundaries

```rust
// ✅ CORRECT: Context flows through function calls
fn process_with_context(block: L1Block, ctx: TraceContext) -> Result<()> {
    let span = info_span!(
        "process_block",
        component = "asm_worker",
        req_id = %ctx.short_id(),
    );
    let _enter = span.enter();

    // Context propagates automatically to child operations
    child_operation(&block)?;  // Inherits span context
    Ok(())
}

// ❌ INCORRECT: Losing context
fn process_no_context(block: L1Block) -> Result<()> {
    // BAD: No span, no context propagation
    child_operation(&block)?;
    Ok(())
}
```

### Rule 4: Use Appropriate Span Levels

```rust
// info_span!  - Use for: User-facing operations, RPC handlers, service message handlers
// debug_span! - Use for: Internal operations, sub-steps, helper functions
// trace_span! - Use for: Very fine-grained operations (rarely needed)
// error_span! - Don't use (use error! inside spans instead)

// ✅ CORRECT usage:
info_span!("rpc_handler", ...)          // Public API
debug_span!("validate_signature", ...)   // Internal step
trace_span!("hash_calculation", ...)     // Very fine detail

// ❌ INCORRECT: Using info for everything
info_span!("hash_bytes", ...)  // BAD: Too verbose for production
```

---

## Error Handling & Logging

### Rule 5: Always Log Errors With Context

```rust
// ✅ CORRECT: Structured error with full context
fn process_block(block: &L1Block) -> Result<()> {
    self.validate(block).map_err(|e| {
        error!(
            error = %e,
            error_type = ?std::any::type_name_of_val(&e),
            l1_height = block.height,
            l1_block = %block.id,
            component = "asm_worker",
            "block validation failed"
        );
        e
    })?;

    Ok(())
}

// ❌ INCORRECT: Generic error without context
fn process_block(block: &L1Block) -> Result<()> {
    self.validate(block)?;  // BAD: Silent failure
    Ok(())
}

// ❌ INCORRECT: Error message without structured fields
fn process_block(block: &L1Block) -> Result<()> {
    self.validate(block).map_err(|e| {
        error!("validation failed: {}", e);  // BAD: Not filterable
        e
    })?;
    Ok(())
}
```

### Rule 6: Use anyhow::Context for Error Chains

```rust
// ✅ CORRECT: Error context at each layer
fn outer_operation(block: &L1Block) -> anyhow::Result<()> {
    inner_operation(block)
        .context("outer operation failed")
        .with_context(|| format!(
            "block_height={}, block_id={}",
            block.height,
            block.id
        ))?;
    Ok(())
}

fn inner_operation(block: &L1Block) -> anyhow::Result<()> {
    validate_signature(block)
        .context("signature validation failed")?;
    Ok(())
}

// When this fails, error chain is:
// outer operation failed: block_height=100, block_id=abc123
//   Caused by: signature validation failed
//   Caused by: invalid signature format
```

### Rule 7: Don't Log And Return Errors

```rust
// ✅ CORRECT: Log at the handling site, not at creation
fn process_block(block: &L1Block) -> Result<()> {
    validate_block(block)?;  // Don't log here
    Ok(())
}

fn handle_request() {
    match process_block(&block) {
        Ok(_) => info!("block processed"),
        Err(e) => {
            error!(error = %e, "failed to process block");  // Log here
        }
    }
}

// ❌ INCORRECT: Double logging
fn process_block(block: &L1Block) -> Result<()> {
    validate_block(block).map_err(|e| {
        error!("validation failed: {}", e);  // BAD: Logs here
        e  // And caller also logs = duplicate
    })?;
    Ok(())
}
```

---

## RPC Instrumentation

### Rule 8: All RPC Methods Must Accept trace_ctx

```rust
// ✅ CORRECT: RPC method with trace context
#[rpc(server, namespace = "strata")]
pub trait StrataApi {
    #[method(name = "getBlocksAtIdx")]
    async fn get_blocks_at_idx(
        &self,
        idx: u64,
        trace_ctx: Option<TraceContext>,  // ✅ REQUIRED
    ) -> RpcResult<Vec<HexBytes32>>;
}

// ❌ INCORRECT: Missing trace_ctx
#[rpc(server, namespace = "strata")]
pub trait StrataApi {
    #[method(name = "getBlocksAtIdx")]
    async fn get_blocks_at_idx(
        &self,
        idx: u64,
        // ❌ Missing trace_ctx parameter
    ) -> RpcResult<Vec<HexBytes32>>;
}
```

### Rule 9: RPC Handlers Must Create Spans

```rust
// ✅ CORRECT: RPC handler with proper instrumentation
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
        otel.kind = "server",  // OpenTelemetry semantic convention
        idx,
    );

    async move {
        info!("handling getBlocksAtIdx");
        self.storage.l2().get_blocks_at_slot(idx).await
            .map(|blocks| blocks.into_iter().map(HexBytes32).collect())
            .map_err(to_jsonrpsee_error)
    }
    .instrument(span)
    .await
}

// ❌ INCORRECT: No span, no context injection
async fn get_blocks_at_idx(
    &self,
    idx: u64,
    trace_ctx: Option<TraceContext>,
) -> RpcResult<Vec<HexBytes32>> {
    // BAD: Ignores trace_ctx, no span created
    self.storage.l2().get_blocks_at_slot(idx).await
        .map(|blocks| blocks.into_iter().map(HexBytes32).collect())
        .map_err(to_jsonrpsee_error)
}
```

### Rule 10: Use InstrumentedRpcClient for Outbound Calls

```rust
// ✅ CORRECT: Using instrumented client
let rpc_client = Arc::new(InstrumentedRpcClient::new(
    client,
    "target-service-name",
));

let result = rpc_client
    .call_with_trace("method_name", params, Some(trace_ctx))
    .await?;

// ❌ INCORRECT: Direct client usage
let result = client.request("method_name", params).await?;
// BAD: No automatic tracing, loses correlation
```

---

## Performance Considerations

### Rule 11: Rate-Limit High-Frequency Logs

```rust
// ✅ CORRECT: Rate-limited debug in loop
loop {
    debug_ratelimited!(
        "duty_extractor_loop",  // Static key
        cnt = duties.len(),
        "processing duties"
    );

    process_duty(duty);
}

// ❌ INCORRECT: Unrestricted logging in loop
loop {
    debug!(cnt = duties.len(), "processing duties");  // BAD: Spam
    process_duty(duty);
}
```

### Rule 12: Use Sampling for Hot Paths

```rust
// ✅ CORRECT: Sample 1 in 100 operations
let mut counter = 0;
for item in items {
    counter += 1;
    if counter % 100 == 0 {
        debug!(
            iterations = counter,
            processed = results.len(),
            "processing progress"
        );
    }
    process_item(item);
}

// ❌ INCORRECT: Log every iteration
for item in items {
    debug!("processing item");  // BAD: Spam if items.len() > 1000
    process_item(item);
}
```

### Rule 13: Skip Expensive Fields When Appropriate

```rust
// ✅ CORRECT: Skip expensive serialization in production
#[instrument(
    skip_all,  // Skip all parameters by default
    fields(
        component = "asm_worker",
        l1_height = block.height,
        // Don't serialize the entire block
    )
)]
async fn process_block(block: L1Block, ctx: TraceContext) -> Result<()> {
    // ...
}

// ❌ INCORRECT: Serialize large structures
#[instrument(fields(block = ?block))]  // BAD: Expensive
async fn process_block(block: L1Block) -> Result<()> {
    // ...
}
```

---

## Real Examples From Production Logs

This section shows actual problems from `/functional-tests/_dd/3-12-fyxzs/_crash_duty_sign_block/sequencer/service.log`.

### Example 1: ASM Worker - Abbreviated IDs Break Correlation

**❌ Current Production Log:**
```
2025-12-03T07:13:37.782398Z  INFO handlemsg: strata_asm_worker::service: ASM found pivot anchor state pivot_block=100@30eb..7e34 service=asm_worker input=L1BlockCommitment(height=100, blkid=347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30)
2025-12-03T07:13:37.798492Z  INFO handlemsg: strata_asm_worker::service: ASM found pivot anchor state pivot_block=100@30eb..7e34 service=asm_worker input=L1BlockCommitment(height=101, blkid=31da6bb9d50589812a7afcb6800240058057f53b72cf98b9579e035f1f041383)
2025-12-03T07:13:37.798513Z  INFO handlemsg: strata_asm_worker::service: ASM transition attempt block_id=101@8313..da31 service=asm_worker input=L1BlockCommitment(height=101, blkid=31da6bb9d50589812a7afcb6800240058057f53b72cf98b9579e035f1f041383)
```

**Problems:**
1. **Abbreviated IDs**: `30eb..7e34` and `8313..da31` are ungrepable
2. **No request ID**: Can't distinguish block 100 vs 101 processing
3. **Inconsistent**: Full ID in `input=` but abbreviated in `pivot_block=` and `block_id=`
4. **Interleaved**: Block 100 and 101 logs are mixed together

**Try This:**
```bash
$ grep "30eb..7e34" service.log  # FAILS - the ".." breaks grep
$ grep "8313..da31" service.log  # FAILS
```

**✅ How It Should Look:**
```
2025-12-03T07:13:37.782398Z  INFO handlemsg: strata_asm_worker::service: ASM found pivot anchor state component=asm_worker req_id=a1b2c3d4 l1_height=100 l1_block=347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30
2025-12-03T07:13:37.798492Z  INFO handlemsg: strata_asm_worker::service: ASM found pivot anchor state component=asm_worker req_id=e5f6g7h8 l1_height=101 l1_block=31da6bb9d50589812a7afcb6800240058057f53b72cf98b9579e035f1f041383
2025-12-03T07:13:37.798513Z  INFO handlemsg: strata_asm_worker::service: ASM transition attempt component=asm_worker req_id=e5f6g7h8 l1_height=101 l1_block=31da6bb9d50589812a7afcb6800240058057f53b72cf98b9579e035f1f041383
```

**Now You Can:**
```bash
$ grep "req_id=a1b2c3d4" service.log  # See ALL events for block 100
$ grep "req_id=e5f6g7h8" service.log  # See ALL events for block 101
$ grep "l1_block=347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30" service.log  # Works!
```

### Example 2: Fork Choice Manager - Abbreviated + Verbose State Dumps

**❌ Current Production Log:**
```
2025-12-03T07:13:37.954394Z  INFO strata_consensus_logic::fork_choice_manager: processing new block slot=1 blkid=a34be0..202667
2025-12-03T07:13:37.954427Z  INFO strata_consensus_logic::fork_choice_manager: handling new block blkid=a34be0..202667 slot=1
2025-12-03T07:13:38.008418Z  INFO strata_consensus_logic::fork_choice_manager: new chain tip tip_blkid=a34be0..202667
2025-12-03T07:13:38.010835Z DEBUG strata_consensus_logic::fork_choice_manager: fcm_state.chain_tracker=UnfinalizedBlockTracker { finalized_epoch: EpochCommitment(epoch=0, last_slot=0, last_blkid=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d), pending_table: {a34be03140eb522ab7dba505d59de93818d7895d5b3f79a7a979fd1a86202667: BlockEntry { slot: 1, parent: aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d, children: {} }, ... } }
```

**Problems:**
1. **Abbreviated ID**: `a34be0..202667` is ungrepable
2. **Verbose state dump**: Multi-line debug struct is hard to read
3. **No component tag**: Just module path `strata_consensus_logic::fork_choice_manager`
4. **No request ID**: Can't correlate with the L2 block that triggered this

**✅ How It Should Look:**
```
2025-12-03T07:13:37.954394Z  INFO strata_consensus_logic::fork_choice_manager: processing new block component=fork_choice_manager req_id=f7e3a1c2 l2_slot=1 l2_block=a34be03140eb522ab7dba505d59de93818d7895d5b3f79a7a979fd1a86202667
2025-12-03T07:13:37.954427Z  INFO strata_consensus_logic::fork_choice_manager: handling new block component=fork_choice_manager req_id=f7e3a1c2 l2_slot=1 l2_block=a34be03140eb522ab7dba505d59de93818d7895d5b3f79a7a979fd1a86202667
2025-12-03T07:13:38.008418Z  INFO strata_consensus_logic::fork_choice_manager: new chain tip component=fork_choice_manager req_id=f7e3a1c2 l2_slot=1 l2_block=a34be03140eb522ab7dba505d59de93818d7895d5b3f79a7a979fd1a86202667
2025-12-03T07:13:38.010835Z DEBUG strata_consensus_logic::fork_choice_manager: updated chain tracker component=fork_choice_manager req_id=f7e3a1c2 finalized_epoch=0 pending_blocks=2 unfinalized_tips=1
```

**Improvement:** State summary instead of full dump, consistent IDs, component tag, request correlation.

### Example 3: Duty Extractor - Log Spam Without Rate Limiting

**❌ Current Production Log:**
```
2025-12-03T07:13:37.804636Z DEBUG strata_sequencer::duty::extractor: have some duties cnt=1
2025-12-03T07:13:37.838785Z DEBUG strata_sequencer::duty::extractor: have some duties cnt=1
2025-12-03T07:13:37.896002Z DEBUG strata_sequencer::duty::extractor: have some duties cnt=1
2025-12-03T07:13:37.913571Z DEBUG strata_sequencer::duty::extractor: have some duties cnt=1
2025-12-03T07:13:37.994291Z DEBUG strata_sequencer::duty::extractor: have some duties cnt=1
2025-12-03T07:13:38.334613Z DEBUG strata_sequencer::duty::extractor: have some duties cnt=1
2025-12-03T07:13:38.334606Z DEBUG strata_sequencer::duty::extractor: have some duties cnt=1
2025-12-03T07:13:38.452650Z DEBUG strata_sequencer::duty::extractor: have some duties cnt=1
... (appears 100+ times in 2 seconds)
```

**Problems:**
1. **Log spam**: Same message every ~50ms
2. **No rate limiting**: Appears 100+ times
3. **No useful info**: Doesn't say WHICH duties or what they are
4. **No component tag**: Just module path

**✅ How It Should Look:**
```
// Using rate limiting:
2025-12-03T07:13:37.804636Z DEBUG strata_sequencer::duty::extractor: processing duties component=duty_extractor duties_processed=100 last_5s
2025-12-03T07:13:42.834613Z DEBUG strata_sequencer::duty::extractor: processing duties component=duty_extractor duties_processed=115 last_5s

// Or using sampling (log every 100th):
2025-12-03T07:13:37.804636Z DEBUG strata_sequencer::duty::extractor: processing duty component=duty_extractor iteration=100 duty_type=SignBlock total_processed=100
2025-12-03T07:13:38.452650Z DEBUG strata_sequencer::duty::extractor: processing duty component=duty_extractor iteration=200 duty_type=SignBlock total_processed=200
```

**Code Fix:**
```rust
// BEFORE (spams logs):
loop {
    debug!(cnt = duties.len(), "have some duties");
    process_duty(duty);
}

// AFTER (rate limited):
loop {
    debug_ratelimited!(
        "duty_extractor_loop",  // Static key
        component = "duty_extractor",
        cnt = duties.len(),
        "processing duties"
    );
    process_duty(duty);
}
```

### Example 4: Bail Manager - No Error Context

**❌ Current Production Log:**
```
2025-12-03T07:13:44.958568Z  WARN strata_common::bail_manager: tripped bail interrupt, exiting... ctx=duty_sign_block
```

**Problems:**
1. **No WHY**: What triggered the bail?
2. **No error chain**: What failed before this?
3. **No correlation**: Can't trace back to the operation that failed
4. **No caller info**: Who called `check_bail_trigger`?

**To Understand This, You Need To:**
1. Manually search backwards in logs
2. Guess which operation failed
3. Hope there's an error log somewhere

**✅ How It Should Look:**
```
2025-12-03T07:13:44.956000Z ERROR strata_sequencer::duty::executor: RPC call failed component=sequencer_client req_id=f7e3a1c2 rpc_method=getBlockTemplate error="connection timeout after 5s" duty_id=duty_123
2025-12-03T07:13:44.957000Z ERROR strata_sequencer::duty::executor: triggering bail interrupt component=sequencer_client req_id=f7e3a1c2 reason="RPC timeout" operation="signing block template" duty_id=duty_123 caller=duty_executor.rs:145
2025-12-03T07:13:44.958568Z  WARN strata_common::bail_manager: tripped bail interrupt component=bail_manager req_id=f7e3a1c2 check_ctx=duty_sign_block bail_target=duty_sign_block caller=sequencer.rs:89
2025-12-03T07:13:44.958600Z ERROR strata_common::bail_manager: BAIL INTERRUPT MATCH - EXITING component=bail_manager req_id=f7e3a1c2 ctx=duty_sign_block
```

**Now You Can:**
```bash
$ grep "req_id=f7e3a1c2" service.log
# Shows the complete error chain:
# 1. RPC call timeout
# 2. Bail trigger with reason
# 3. Bail check match
# 4. Exit
```

### Example 5: Error Without Structured Context

**❌ Current Production Log:**
```
2025-12-03T07:13:39.035356Z  WARN strata_sequencer::checkpoint::checkpoint_handle: Failed to update checkpoint update err=SendError(0)
```

**Problems:**
1. **Generic error**: What checkpoint? Which epoch?
2. **No context**: What was being updated?
3. **Unstructured**: `err=SendError(0)` is not filterable
4. **No component tag**
5. **No request ID**

**✅ How It Should Look:**
```rust
// In code:
self.checkpoint_tx.send(update).map_err(|e| {
    error!(
        error = %e,
        error_type = "SendError",
        component = "checkpoint_handle",
        epoch = update.epoch,
        slot = update.slot,
        receiver_dropped = true,
        "failed to send checkpoint update"
    );
    e
})?;
```

**Result:**
```
2025-12-03T07:13:39.035356Z  WARN strata_sequencer::checkpoint::checkpoint_handle: failed to send checkpoint update component=checkpoint_handle epoch=0 slot=2 error_type=SendError receiver_dropped=true error="SendError(0)"
```

### Example 6: L1 Block Reader - Massive Log Spam (137 Logs = 11% of Log File)

**❌ Current Production Log:**
```
2025-12-03T07:13:37.734510Z  INFO strata_btcio::reader::query: accepted new block fetch_height=100 blkid=347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30
2025-12-03T07:13:37.734906Z  INFO strata_btcio::reader::query: accepted new block fetch_height=101 blkid=31da6bb9d50589812a7afcb6800240058057f53b72cf98b9579e035f1f041383
2025-12-03T07:13:37.735312Z  INFO strata_btcio::reader::query: accepted new block fetch_height=102 blkid=4e5918f9219c9ae1dba66e4a494d7944945afeff0d0128924e6e9a9b31c87c99
... (137 consecutive times!)
```

**Problems:**
1. **Massive spam**: 137 "accepted new block" logs in 0.03 seconds
2. **11% of entire log file**: Out of 1214 total lines, 137 are this message
3. **No span**: Each log manually adds fields instead of inheriting from span
4. **No component tag**: Just module path
5. **INFO level for hot path**: Should be DEBUG with rate limiting

**Impact:**
- Makes logs unreadable during L1 catchup
- Drowns out important events
- Wastes storage and bandwidth

**✅ How It Should Look (Span-Based - MINIMAL DEVELOPER WORK):**

```rust
// BEST: Create span ONCE with all context
let span = info_span!(
    "process_l1_batch",
    component = "l1_reader",          // ← Set once
    start_height = start,              // ← Set once
    end_height = end,                  // ← Set once
    blocks_to_process = end - start + 1,
);

async move {
    let mut processed = 0;

    for height in start..=end {
        let block = fetch_block(height).await?;
        processed += 1;

        // Logs inherit component, start_height, end_height automatically!
        // Only log every 10th block
        if processed % 10 == 0 {
            debug!(last_height = height, "batch progress");
        }
    }

    // Final summary inherits all span fields
    info!(blocks_processed = processed, "completed batch");
}
.instrument(span)
.await
```

**Output:**
```
2025-12-03T07:13:37.734510Z  INFO component=l1_reader start_height=100 end_height=222 blocks_to_process=123 process_l1_batch: started
2025-12-03T07:13:37.740425Z DEBUG component=l1_reader start_height=100 end_height=222 last_height=110 batch progress
2025-12-03T07:13:37.750425Z DEBUG component=l1_reader start_height=100 end_height=222 last_height=120 batch progress
... (only 13 logs instead of 137!)
2025-12-03T07:13:37.768382Z  INFO component=l1_reader start_height=100 end_height=222 blocks_processed=123 completed batch
2025-12-03T07:13:37.768382Z  INFO component=l1_reader start_height=100 end_height=222 duration_ms=34 process_l1_batch: closed
```

**Developer Work Required:**
- ✅ Add 6 lines: span creation + instrument wrapper
- ✅ Change 1 line: `info!` → `debug!` with sampling condition
- ✅ **NO manual field addition on every log line!**

**Bonus Benefits:**
- 🎁 Automatic duration tracking (span records start/end time)
- 🎁 All logs automatically have `component`, `start_height`, `end_height`
- 🎁 Can filter all batch logs with `grep "start_height=100 end_height=222"`
- 🎁 Reduces from 137 logs to ~13 logs (10x reduction)

---

**Alternative: If You Can't Use Spans Yet (Batch Summary):**

```rust
// OK: Manual batch summary (more work, no auto-timing)
info!(
    component = "l1_reader",
    start_height = start,
    end_height = end,
    "starting L1 block batch"
);

for height in start..=end {
    let block = fetch_block(height).await?;
    // No individual logging
}

info!(
    component = "l1_reader",
    start_height = start,
    end_height = end,
    blocks_processed = end - start + 1,
    "completed L1 block batch"
);
```

This reduces to 2 logs but requires manually adding fields twice and doesn't give you timing.

### Example 7: CSM Worker - Multi-Kilobyte State Dumps in WARN Logs

**❌ Current Production Log:**
```
2025-12-03T07:13:44.193978Z  WARN handlemsg: strata_common::bail_manager: tripped bail interrupt, exiting... ctx=csm_event service=csm_worker input=AsmWorkerStatus { is_initialized: true, cur_block: Some(L1BlockCommitment(height=233, blkid=1bb92af6823e1b833716f16608ac61668e8880e111fad9bf6a8b76ce22d1ef95)), cur_state: Some(AsmState { state: AnchorState { chain_view: ChainViewState { pow_state: HeaderVerificationState { params: BtcParams(Params { network: Regtest, bip16_time: 1333238400, bip34_height: 100000000, bip65_height: 1351, bip66_height: 1251, rule_change_activation_threshold: 108, miner_confirmation_window: 144, pow_limit: Target(0x7fffff0000000000000000000000000000000000000000000000000000000000), max_attainable_target: Target(0x7fffff0000000000000000000000000000000000000000000000000000000000), pow_target_spacing: 600, pow_target_timespan: 1209600, allow_min_difficulty_blocks: true, no_pow_retargeting: true }), last_verified_block: L1BlockCommitment(height=233, blkid=1bb92af6823e1b833716f16608ac61668e8880e111fad9bf6a8b76ce22d1ef95), next_block_target: 545259519, epoch_start_timestamp: 1296688602, block_timestamp_history: TimestampStore { buffer: [1764746053, 1764746051, 1764746052, 1764746052, 1764746052, 1764746052, 1764746052, 1764746052, 1764746053, 1764746053, 1764746053], head: 1 }, total_accumulated_pow: BtcWork(Work(0x000000000000000000000000000000000000000000000000000000000000010a)) }, manifest_mmr: CompactMmr64 { entries: 133, cap_log2: 64, roots: [[164, 250, 56, 58, 222, 77, 151, 167, 147, 241, 49, 96, 121, 152, 30, 179, 200, 162, 42, 210, 136, 245, 184, 209, 234, 94, 101, 120, 31, 128, 168, 241], [50, 106, 17, 23, 135, 47, 152, 132, 131, 65, 73, 34, 129, 254, 14, 194, 184, 46, 169, 222, 29, 214, 115, 251, 223, 40, 49, 230, 32, 136, 236, 182], [228, 160, 6, 32, 224, 21, 225, 204, 40, 14, 44, 179, 177, 214, 19, 111, 121, 231, 32, 150, 111, 186, 121, 44, 87, 128, 112, 61, 70, 3, 15, 117]] } }, sections: [SectionState { id: 1, data: [0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 77, 90, 39, 73, 139, 109, 13, 243, 252, 224, 76, 49, 251, 126, 210, 196, 160, 234, 124, 21, 38, 176, 44, 24, 188, 178, 251, 198, 136, 93, 89, 97, 1, 0, 0, 0, 1] }, SectionState { id: 2, data: [2, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 151, 211, 243, 7, 126, 243, 191, 139, 45, 77, 193, 177, 36, 157, 135, 76, 67, 226, 168, 144, 1, 250, 237, 85, 19, 105, 189, 175, 76, 139, 187, 52, 1, 0, 0, 0, 137, 74, 61, 153, 192, 137, 4, 201, 76, 62, 120, 90, 6, 234, 44, 20, 218, 239, 223, 25, 213, 243, 56, 71, 37, 190, 217, 57, 26, 183, 229, 168, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 255, 140, 132, 175, 241, 101, 178, 190, 13, 51, 163, 216, 60, 54, 80, 10, 247, 74, 26, 187, 207, 11, 178, 49, 97, 98, 53, 159, 170, 227, 182, 87, 96, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 202, 154, 59, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] }] }, logs: [] }) }
```

**Problems:**
1. **Multi-KB log line**: This single log is ~2,400 characters (2.4 KB)
2. **Completely unreadable**: Debug struct dump with nested arrays
3. **No summary**: Just raw `{:?}` formatting
4. **Breaks log parsers**: Single line is too large for many tools
5. **Useless for debugging**: Can't understand state at a glance

**Impact:**
- Log viewers truncate or crash
- Impossible to read in terminal
- No useful information density
- Wastes storage

**✅ How It Should Look:**
```
2025-12-03T07:13:44.193978Z  WARN handlemsg: strata_common::bail_manager: tripped bail interrupt component=bail_manager req_id=xyz123 check_ctx=csm_event service=csm_worker trigger_state=asm_status asm_initialized=true asm_height=233 asm_block=1bb92af6823e1b833716f16608ac61668e8880e111fad9bf6a8b76ce22d1ef95 asm_mmr_entries=133 asm_sections=2
```

**Code Fix:**
```rust
// BEFORE (crates/common/src/bail_manager.rs):
warn!(
    ctx = check_ctx,
    service = ?service_name,
    input = ?input,  // ❌ Dumps entire struct
    "tripped bail interrupt, exiting..."
);

// AFTER:
warn!(
    component = "bail_manager",
    req_id = %trace_ctx.short_id(),
    check_ctx = check_ctx,
    service = service_name,
    // Extract only relevant fields from input:
    trigger_state = "asm_status",
    asm_initialized = input.is_initialized,
    asm_height = input.cur_block.as_ref().map(|b| b.height()),
    asm_block = input.cur_block.as_ref().map(|b| b.blkid().to_string()),
    asm_mmr_entries = input.cur_state.as_ref().map(|s| s.state.chain_view.manifest_mmr.entries),
    "tripped bail interrupt"
);
```

**Principle:** Never use `?input` or `{:?}` for complex types in production logs. Extract and log only the identifying fields.

### Example 8: Startup Logs Without Component Tags

**❌ Current Production Log:**
```
2025-12-03T07:13:37.719066Z  INFO strata_client: startup: genesis params in sync with reth
2025-12-03T07:13:37.720641Z  INFO strata_client: startup: last matured block: 347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30
2025-12-03T07:13:37.712294Z  INFO strata_client: init finished, starting main tasks
```

**Problems:**
1. **No component tag**: Just "strata_client" module path
2. **String prefix "startup:"**: Not filterable
3. **No structured fields**: Block ID is in message text
4. **Can't filter**: How do you find all startup logs?

**✅ How It Should Look:**
```
2025-12-03T07:13:37.719066Z  INFO strata_client: genesis params synced component=client_init phase=startup reth_synced=true
2025-12-03T07:13:37.720641Z  INFO strata_client: last matured block component=client_init phase=startup l1_height=100 l1_block=347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30
2025-12-03T07:13:37.712294Z  INFO strata_client: init finished component=client_init phase=complete
```

**Now You Can:**
```bash
$ grep "component=client_init" service.log  # All client init logs
$ grep "phase=startup" service.log          # All startup phase logs
```

---

## Anti-Patterns & Code Smells

### ❌ Anti-Pattern 0: Manual Fields Instead of Spans (MOST COMMON)

**Bad: Repeating context on every log:**
```rust
fn process_batch(start: u64, end: u64) -> Result<()> {
    info!(component = "worker", start, end, "starting batch");

    for i in start..=end {
        // ❌ Repeating component, start, end on every log
        debug!(component = "worker", start, end, i, "processing item");
        process_item(i)?;
    }

    info!(component = "worker", start, end, "completed batch");
    Ok(())
}
```

**Good: Span with automatic inheritance:**
```rust
fn process_batch(start: u64, end: u64) -> Result<()> {
    let span = info_span!(
        "process_batch",
        component = "worker",  // ← Set once
        start,                 // ← Set once
        end,                   // ← Set once
    );
    let _enter = span.enter();

    info!("starting batch");  // ← Automatically has component, start, end

    for i in start..=end {
        debug!(i, "processing item");  // ← Automatically has component, start, end
        process_item(i)?;
    }

    info!("completed batch");  // ← Automatically has component, start, end
    Ok(())
}
```

**Why this matters:**
- **Less typing:** Set context once, not N times
- **Can't forget:** No "oops I forgot component on this log"
- **Refactoring:** Change field names in one place
- **Free timing:** Span automatically records duration

---

### ❌ Anti-Pattern 1: Abbreviated Identifiers

**From Production:**
```
pivot_block=100@30eb..7e34    // ❌ Can't grep for "30eb..7e34"
block_id=101@8313..da31        // ❌ Can't grep for "8313..da31"
blkid=a34be0..202667           // ❌ Can't grep for "a34be0..202667"
```

**Fixed:**
```rust
// GOOD: Full identifier on span
let span = info_span!(
    "process_block",
    component = "asm_worker",
    l1_height = 100,
    l1_block = "347e16b7626c9ae343b2c60beb93d5d9821ba01b7f560ccc3947419d8000eb30",
);
```

### Summary: What Needs to Fix in Current Codebase

Based on the production log analysis (1,214 log lines examined):

| Component | File Location | Issues Found | Examples | Fix Priority |
|-----------|--------------|--------------|----------|--------------|
| ASM Worker | `crates/asm/worker/src/service.rs` | Abbreviated IDs, no req_id, inconsistent fields | Example 1 | HIGH |
| Fork Choice Manager | `crates/consensus-logic/src/fork_choice_manager.rs` | Abbreviated IDs, verbose state dumps, no component tag | Example 2 | HIGH |
| Duty Extractor | `crates/sequencer/src/duty/extractor.rs` | Log spam (100+/sec), no rate limiting | Example 3 | MEDIUM |
| Bail Manager | `crates/common/src/bail_manager.rs` | No error context, no caller info, multi-KB state dumps | Examples 4, 7 | HIGH |
| Checkpoint Handle | `crates/sequencer/src/checkpoint/checkpoint_handle.rs` | Generic errors, no structured context | Example 5 | MEDIUM |
| L1 Block Reader | `crates/btcio/src/reader/query.rs` | **137 log spam (11% of entire log file)**, no batching | Example 6 | HIGH |
| Client Init | `crates/client/src/main.rs` | No component tags, string prefixes, unstructured fields | Example 8 | LOW |

#### Actionable Steps

**Week 1: Fix High-Priority Issues**

1. **ASM Worker** (`crates/asm/worker/src/service.rs:81-100`):
   ```rust
   // Change this:
   info!(%pivot_block, "ASM found pivot anchor state");

   // To this:
   info!(
       component = "asm_worker",
       req_id = %trace_ctx.short_id(),
       l1_height = pivot_block.height(),
       l1_block = %pivot_block.blkid(),
       "ASM found pivot anchor state"
   );
   ```

2. **Fork Choice Manager** (`crates/consensus-logic/src/fork_choice_manager.rs:954-1010`):
   ```rust
   // Change this:
   info!(slot, blkid = %blkid.abbrev(), "processing new block");

   // To this:
   info!(
       component = "fork_choice_manager",
       req_id = %ctx.short_id(),
       l2_slot = slot,
       l2_block = %blkid,  // Full ID
       "processing new block"
   );
   ```

3. **Bail Manager** (`crates/common/src/bail_manager.rs:29-36`):
   - Add `BailTriggerContext` struct with reason, operation, state
   - Add `#[track_caller]` to capture source location
   - Log error chain before triggering bail
   - **Extract summary fields instead of dumping entire structs** (see Example 7)
   - See "Part 7: Fix the Bail Manager" in `improvements.md`

4. **L1 Block Reader** (`crates/btcio/src/reader/query.rs`):
   ```rust
   // Wrap the loop in a span:
   let span = info_span!(
       "process_l1_batch",
       component = "l1_reader",  // ← Set once, inherited by all logs
       start_height = start,
       end_height = end,
   );

   async move {
       for height in start..=end {
           let block = fetch_block(height).await?;
           // Only log every 10th block - no manual fields needed!
           if height % 10 == 0 {
               debug!(last_height = height, "batch progress");
           }
       }
       info!(blocks_processed = end - start + 1, "completed batch");
   }
   .instrument(span)
   .await
   ```

   **Result:** 137 logs → ~13 logs (10x reduction), automatic timing, zero manual field duplication

**Week 2: Add Rate Limiting**

5. **Duty Extractor** (`crates/sequencer/src/duty/extractor.rs`):
   ```rust
   // Add rate limiting module
   use strata_common::rate_limited_log::debug_ratelimited;

   // In hot loop:
   debug_ratelimited!(
       "duty_extractor_loop",
       component = "duty_extractor",
       cnt = duties.len(),
       "processing duties"
   );
   ```

6. **Create** `crates/common/src/rate_limited_log.rs`:
   - See "Part 9: Loop Detection & Rate Limiting" in `improvements.md`

**Week 3-4: Systematic Rollout**

7. Add `component` field to all remaining workers
8. Add `req_id` propagation to all service message handlers
9. Audit and fix all abbreviated IDs throughout codebase
10. Add structured error context to all `map_err` calls
11. Fix client initialization logs (Example 8) with component tags

---

### ❌ Anti-Pattern 2: Inconsistent Field Names

```rust
// BAD: Different names for same concept
info!(block_height = 100, ...);
info!(l1_height = 100, ...);
info!(height = 100, ...);

// GOOD: Consistent naming
info!(l1_height = 100, ...);
info!(l1_height = 101, ...);
```

### ❌ Anti-Pattern 3: Missing Component Field

```rust
// BAD: Can't filter by subsystem
info_span!("process_block", l1_height = 100);

// GOOD: Always include component
info_span!(
    "process_block",
    component = "asm_worker",  // ← REQUIRED
    l1_height = 100
);
```

### ❌ Anti-Pattern 4: Span Soup (Too Many Spans)

```rust
// BAD: Excessive granularity
async fn process_block() {
    let span1 = info_span!("step1");
    step1().instrument(span1).await;

    let span2 = info_span!("step2");
    step2().instrument(span2).await;

    let span3 = info_span!("step3");
    step3().instrument(span3).await;
    // BAD: Creates 3 spans for trivial steps
}

// GOOD: One span for the operation
async fn process_block() {
    let span = info_span!("process_block", ...);
    async move {
        step1();
        step2();
        step3();
    }
    .instrument(span)
    .await
}
```

### ❌ Anti-Pattern 5: Stringly-Typed Fields

```rust
// BAD: String when you should use typed field
info!(status = "processing", ...);

// GOOD: Use structured data
info!(status = ?BlockStatus::Processing, ...);

// GOOD: Or use separate fields
info!(is_processing = true, ...);
```

---

## Clippy Rules & Enforcement

### Custom Clippy Rules

Add to `.clippy.toml`:

```toml
# Enforce that all functions with Result<> should have error context
# (requires custom lint - see below)

# Warn on format! in hot paths
too-many-arguments-threshold = 5

# Enforce explicit error types
result-large-err = 128
```

### Custom Lints (To Be Implemented)

Create `lints/src/lib.rs`:

```rust
// Custom lint: Ensure spans have required fields
declare_clippy_lint! {
    pub MISSING_SPAN_COMPONENT,
    correctness,
    "span created without 'component' field"
}

// Custom lint: Detect abbreviated block IDs
declare_clippy_lint! {
    pub ABBREVIATED_BLOCK_ID,
    correctness,
    "block ID abbreviated with '..'"
}

// Custom lint: Detect logging in loops without rate limiting
declare_clippy_lint! {
    pub UNTHROTTLED_LOOP_LOG,
    perf,
    "logging inside loop without rate limiting"
}
```

### Enforcement via CI

Add to `.github/workflows/ci.yml`:

```yaml
- name: Check Logging Standards
  run: |
    # Check for abbreviated block IDs
    ! git grep -n '\.\.[0-9a-f]\{6\}' -- '*.rs' || {
      echo "ERROR: Found abbreviated block IDs"
      exit 1
    }

    # Check for spans without component field
    ./scripts/check_span_standards.sh

    # Check for unrestricted logging in loops
    ./scripts/check_loop_logging.sh
```

### Pre-Commit Hook

Create `.git/hooks/pre-commit`:

```bash
#!/bin/bash

# Check for common anti-patterns
if git diff --cached --name-only | grep '\.rs$' > /dev/null; then
    # Check for abbreviated block IDs
    if git diff --cached | grep -E '\.\.[0-9a-f]{6}' > /dev/null; then
        echo "ERROR: Found abbreviated block IDs (use full IDs)"
        exit 1
    fi

    # Check for 'blkid' instead of 'l1_block' or 'l2_block'
    if git diff --cached | grep 'blkid\s*=' > /dev/null; then
        echo "WARNING: Use 'l1_block' or 'l2_block' instead of 'blkid'"
    fi
fi
```

---

## Testing Requirements

### Rule 14: Test Trace Propagation

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Layer;

    #[tokio::test]
    async fn test_request_correlation() {
        // Setup test subscriber
        let (subscriber, handle) = tracing_subscriber::fmt()
            .with_test_writer()
            .finish()
            .with_handles();

        tracing::subscriber::set_global_default(subscriber).unwrap();

        // Create trace context
        let ctx = TraceContext::new_root();
        let req_id = ctx.request_id.clone();

        // Execute operation
        process_block(block, ctx).await.unwrap();

        // Verify req_id appears in all logs
        let logs = handle.collect();
        assert!(logs.iter().all(|log| log.contains(&req_id)));
    }
}
```

### Rule 15: Test Error Context Propagation

```rust
#[test]
fn test_error_context_chain() {
    let result = outer_operation(&block);

    match result {
        Err(e) => {
            let error_chain = format!("{:?}", e);
            // Verify error context is preserved
            assert!(error_chain.contains("outer operation failed"));
            assert!(error_chain.contains("block_height=100"));
            assert!(error_chain.contains("inner error"));
        }
        Ok(_) => panic!("expected error"),
    }
}
```

---

## Code Review Checklist

### For All Code Changes

- [ ] Every span has a `component` field
- [ ] Every operation span has a `req_id` field
- [ ] All entity IDs are full, never abbreviated
- [ ] All RPC methods accept `trace_ctx: Option<TraceContext>`
- [ ] All RPC handlers create spans with proper fields
- [ ] Errors are logged with structured fields, not just messages
- [ ] High-frequency logs are rate-limited or sampled
- [ ] No sensitive data (keys, credentials) in logs
- [ ] Test coverage includes trace propagation
- [ ] Documentation updated if adding new components

### For Service Workers

- [ ] `process_input` creates a span with all required fields
- [ ] Trace context is created for each incoming message
- [ ] Child operations propagate the trace context
- [ ] State transitions are logged with old/new state

### For RPC Changes

- [ ] Server: Injects trace context at handler entry
- [ ] Server: Creates span with `otel.kind = "server"`
- [ ] Client: Uses `InstrumentedRpcClient` wrapper
- [ ] Client: Propagates trace context in calls
- [ ] Both: Log RPC method name and duration

### For Error Handling

- [ ] Errors include structured fields (not just messages)
- [ ] Error chains use `.context()` / `.with_context()`
- [ ] Errors logged at handling site, not at creation
- [ ] Error type is included in logs (`error_type` field)

---

## Examples: Before & After

### Example 1: Service Worker

**❌ Before:**

```rust
fn process_input(
    state: &mut State,
    block: &L1BlockCommitment,
) -> anyhow::Result<Response> {
    info!("processing block");

    let result = state.validate(block);
    if result.is_err() {
        return Err(anyhow!("validation failed"));
    }

    Ok(Response::Continue)
}
```

**✅ After:**

```rust
fn process_input(
    state: &mut State,
    block: &L1BlockCommitment,
) -> anyhow::Result<Response> {
    let trace_ctx = TraceContext::new_root()
        .with_baggage("l1_height", block.height().to_string());

    let span = info_span!(
        "handlemsg",
        component = "asm_worker",
        service = "asm_worker",
        req_id = trace_ctx.short_id(),
        l1_height = block.height(),
        l1_block = %block.blkid(),
        trigger = "l1_block",
    );
    let _enter = span.enter();

    info!("processing L1 block");

    state.validate(block).map_err(|e| {
        error!(
            error = %e,
            l1_height = block.height(),
            l1_block = %block.blkid(),
            "block validation failed"
        );
        e
    })?;

    info!("L1 block processed successfully");
    Ok(Response::Continue)
}
```

### Example 2: RPC Handler

**❌ Before:**

```rust
async fn get_block(&self, height: u64) -> RpcResult<Block> {
    self.storage.get_block(height).await
        .map_err(|e| jsonrpsee::core::Error::from(e))
}
```

**✅ After:**

```rust
async fn get_block(
    &self,
    height: u64,
    trace_ctx: Option<TraceContext>,
) -> RpcResult<Block> {
    let trace_ctx = trace_ctx.unwrap_or_else(TraceContext::new_root);
    inject_trace_context(&trace_ctx);

    let span = info_span!(
        "rpc_handler",
        component = "strata_rpc",
        rpc_method = "getBlock",
        req_id = trace_ctx.short_id(),
        otel.kind = "server",
        l2_slot = height,
    );

    async move {
        info!("handling getBlock");

        self.storage.get_block(height).await
            .map_err(|e| {
                error!(
                    error = %e,
                    l2_slot = height,
                    "failed to retrieve block"
                );
                jsonrpsee::core::Error::from(e)
            })
    }
    .instrument(span)
    .await
}
```

### Example 3: Error Handling

**❌ Before:**

```rust
fn validate_block(block: &Block) -> Result<()> {
    if !check_signature(block) {
        return Err(anyhow!("invalid signature"));
    }
    Ok(())
}
```

**✅ After:**

```rust
fn validate_block(block: &Block) -> Result<()> {
    check_signature(block)
        .context("signature validation failed")
        .with_context(|| format!(
            "block validation failed: l2_slot={}, l2_block={}",
            block.slot(),
            block.id()
        ))?;
    Ok(())
}
```

---

## Quick Reference Card

### Span Creation Template

```rust
let span = info_span!(
    "operation_name",
    component = "subsystem_name",        // REQUIRED
    req_id = %trace_ctx.short_id(),     // REQUIRED
    entity_type = entity_value,          // REQUIRED (l1_height, l2_slot, etc.)
    entity_id = %entity_id,              // REQUIRED (full ID)
);
```

### Error Logging Template

```rust
.map_err(|e| {
    error!(
        error = %e,                      // REQUIRED
        error_type = ?type_name_of_val(&e),
        component = "subsystem",         // REQUIRED
        entity_field = value,            // Context
        "operation failed"               // Message
    );
    e
})?;
```

### RPC Handler Template

```rust
async fn rpc_method(
    &self,
    params: ParamType,
    trace_ctx: Option<TraceContext>,    // REQUIRED
) -> RpcResult<ReturnType> {
    let trace_ctx = trace_ctx.unwrap_or_else(TraceContext::new_root);
    inject_trace_context(&trace_ctx);

    let span = info_span!(
        "rpc_handler",
        component = "rpc_server_name",
        rpc_method = "methodName",
        req_id = trace_ctx.short_id(),
        otel.kind = "server",
    );

    async move {
        // Implementation
    }
    .instrument(span)
    .await
}
```

---

## Enforcement Timeline

### Immediate (All New Code)
- All new spans MUST have `component` field
- All new RPC methods MUST accept `trace_ctx`
- All new errors MUST use structured logging

### Phase 1 (2 Weeks)
- Add custom clippy lints
- Set up CI checks for standards
- Add pre-commit hooks

### Phase 2 (1 Month)
- Refactor existing code to add `component` fields
- Update existing RPC methods with `trace_ctx`
- Add rate limiting to identified hot paths

### Phase 3 (2 Months)
- All code complies with standards
- CI enforcement enabled (blocking)
- Standards become mandatory for merges

---

## Verification and Challenges

### These Standards Are Proposals, Not Gospel

While these standards are based on industry best practices (see References section above), they are **proposals for our specific context**. They should be:

1. **Questioned**: If a standard doesn't make sense for your use case, challenge it
2. **Measured**: We should measure actual overhead and adjust if needed
3. **Evolved**: As we learn what works in production, update these standards

### How to Validate These Standards

#### For Request IDs (Principle 1):
- **Validate by**: Attempt to debug a production issue without correlation
- **Success metric**: Can you reconstruct a request's lifecycle from logs?
- **Challenge**: If correlation adds >5% overhead, we need to sample

#### For Component Tags (Principle 2):
- **Validate by**: Try to filter logs by subsystem using current approach
- **Success metric**: Can you isolate one component's activity in <1 minute?
- **Challenge**: If tags become inconsistent, we need better linting

#### For Full IDs (Principle 3):
- **Validate by**: Try to grep for an abbreviated ID in production logs
- **Success metric**: Can you find all logs about a specific entity?
- **Challenge**: If log volume becomes prohibitive, add structured field extraction

#### For Rate Limiting (Principle 4):
- **Validate by**: Measure log volume before/after rate limiting
- **Success metric**: Is high-frequency spam reduced without losing signal?
- **Challenge**: If rate limiting hides critical issues, adjust thresholds

### References to Cross-Check

If you're skeptical of any standard here, verify against:

1. **OpenTelemetry Docs**: Do they recommend this pattern?
   - Check: https://opentelemetry.io/docs/specs/semconv/

2. **Your Production Logs**: Does this solve a real problem you've experienced?
   - Ask: "Would this have helped debug [specific incident]?"

3. **Performance Data**: What does actual measurement show?
   - Test: Instrument one service, measure overhead, decide if acceptable

4. **Team Experience**: What do observability experts say?
   - Read: Charity Majors' blog posts on high-cardinality logging
   - Read: Google SRE book chapters on monitoring

### Known Limitations and Trade-offs

**These standards are not zero-cost:**

1. **Performance**: 1-3% CPU overhead, 2-3x log volume
   - **Trade-off**: Accepted for debuggability
   - **Mitigation**: Sampling, rate limiting, async recording

2. **Code Complexity**: 3-5 extra lines per operation for spans
   - **Trade-off**: Accepted for observability
   - **Mitigation**: Macros, helper functions, code generation

3. **Learning Curve**: Team needs to learn tracing concepts
   - **Trade-off**: One-time cost for long-term benefit
   - **Mitigation**: Documentation, examples, code reviews

4. **Storage Costs**: More logs = more storage
   - **Trade-off**: $0.02/GB vs. $100/hour debugging time
   - **Mitigation**: Retention policies, log aggregation, compression

### Adaptation Process

If a standard proves problematic in practice:

1. **Document the problem**: What broke? What was the cost?
2. **Propose alternative**: What would work better?
3. **Test alternative**: Implement in one service, measure results
4. **Update standard**: If alternative is better, update this doc
5. **Announce change**: Notify team so everyone adopts new approach

**Example**: If we find that full block IDs cause log volume to exceed storage budget, we might:
- Alternative 1: Use first 16 chars instead of 6 (still grepable)
- Alternative 2: Store full IDs in structured fields only, abbreviate in messages
- Alternative 3: Increase storage budget (logs are worth the cost)
- Test all three, pick best based on data

---

## Getting Help

- **Questions about standards**: Ask in #observability channel
- **Skeptical of a standard**: Review the References section, then open a discussion
- **Unclear how to instrument**: See examples in this doc or improvements.md
- **Performance concerns**: Measure first, then consult with platform team
- **Standard exceptions**: Document why, get tech lead approval

## Document Updates

This is a living document. Propose changes via:
1. Open an issue with `[standards]` prefix
2. Cite references or data supporting the change
3. Get consensus from 2+ engineers
4. Update this doc with PR
5. Announce changes in #engineering

**All standards should be backed by either:**
- Industry standard (OpenTelemetry, W3C, etc.)
- Measured data from our systems
- Clear reasoning about our specific architecture

---

## Further Reading

If you want to understand the "why" behind these standards:

1. **Start here**: `improvements-in-words.md` - Conceptual explanation
2. **Then read**: `improvements.md` - Implementation details with code
3. **For specific patterns**: This document - Standards and enforcement
4. **For deep dives**: References section above - Primary sources

### Recommended Learning Path

**Day 1**: Read `improvements-in-words.md` to understand the problems

**Week 1**: Skim OpenTelemetry semantic conventions, read Google Dapper paper

**Week 2**: Implement instrumentation in one small service using these standards

**Week 3**: Measure overhead, verify correlation works, adjust if needed

**Month 1**: Roll out to more services, update standards based on learnings

---

**Remember:** Good instrumentation is like comments - future you (debugging at 3am) will thank present you for being thorough. But don't blindly follow standards. Verify they work for your context, measure their impact, and evolve them based on data.

---

## Final Summary: Minimize Your Work

**The Core Pattern (memorize this):**

1. **Create a span** with `component`, `req_id`, and entity IDs
2. **Wrap your work** with `.instrument(span)`
3. **Log inside** without repeating fields - they're inherited

**Example:**
```rust
let span = info_span!("op_name", component = "worker", req_id = %ctx.short_id());
async move {
    info!("starting");  // ← Inherits component & req_id automatically
    do_work()?;
    info!("done");      // ← Inherits component & req_id automatically
}.instrument(span).await
```

**You get for free:**
- ✅ Automatic field inheritance (no duplication)
- ✅ Automatic duration tracking
- ✅ Automatic trace correlation
- ✅ Greppable logs by request ID
- ✅ Filterable logs by component

**What to avoid:**
- ❌ Manually adding `component = "..."` to every log
- ❌ Manually adding `req_id = %...` to every log
- ❌ Logging in tight loops without rate limiting
- ❌ Using `{:?}` debug formatting for large structs
- ❌ Abbreviated IDs that break grep

**Your checklist for every new operation:**
1. [ ] Created a span with `component` and `req_id`?
2. [ ] Used `.instrument(span)` to attach it?
3. [ ] Logs inside are free of repeated context fields?
4. [ ] Entity IDs are full, not abbreviated?
5. [ ] Hot loops are rate-limited or sampled?

**That's it.** Follow this pattern and you'll have world-class observability with minimal effort.

---

**Questions?** See [getting-started.md](./getting-started.md) for concepts, [improvements-in-words.md](./improvements-in-words.md) for problems/solutions, and [improvements.md](./improvements.md) for detailed code examples.
