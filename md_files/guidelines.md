# Observability Guidelines - Mandatory Requirements for Mainnet

**Status:** MANDATORY - All new code and refactors MUST follow these guidelines.

**Why these exist:** The resilience of a system relies on observability. When things break in production, we need to debug quickly. These guidelines ensure we can:
- Trace operations across components
- Filter logs by logical system (not file paths)
- Find performance bottlenecks
- Correlate related events

**Enforcement:** Code reviews MUST check compliance. PRs violating these guidelines will be rejected.

---

## Table of Contents

1. [Foundations: Structured Logging vs Printf-Style Logging](#foundations-structured-logging-vs-printf-style-logging)
   - [What is Structured Logging?](#what-is-structured-logging)
   - [Understanding `%` and `?` Formatters](#understanding-the--and--formatters)
   - [Log Levels: When to Use Each](#log-levels-when-to-use-each)
   - [Common Mistakes](#common-mistakes)
2. [Core Principles (10 Mandatory Rules)](#core-principles)
3. [Quick Checklist for Code Review](#quick-checklist-for-code-review)
4. [Examples: Good vs Bad](#examples-good-vs-bad)
5. [Why These Rules Matter for Mainnet](#why-these-rules-matter-for-mainnet)
6. [Exceptions](#exceptions)

**New to structured logging?** Read the [Foundations](#foundations-structured-logging-vs-printf-style-logging) section first.

**Already know the basics?** Jump to [Core Principles](#core-principles) for the 10 mandatory rules.

---

## Quick Reference Card

**Import first:**
```rust
use tracing::{info, debug, warn, error, info_span};
use strata_common::fields;
```

**Basic structured log:**
```rust
info!(
    {fields::COMPONENT} = "asm_worker",
    {fields::L1_BLOCK} = %block_id,    // % for Display types
    {fields::L1_HEIGHT} = height,      // No formatter for primitives
    "processing block"                 // Message last
);
```

**With span (preferred):**
```rust
let span = info_span!(
    "process_block",
    {fields::COMPONENT} = "asm_worker",
    {fields::L1_BLOCK} = %block_id,
    {fields::L1_HEIGHT} = height,
);

async move {
    info!("started");    // Inherits all span fields
    do_work().await?;
    info!("completed");  // Inherits all span fields
}
.instrument(span)
.await
```

**Error with context:**
```rust
error!(
    {fields::ERROR} = %e,
    {fields::L1_BLOCK} = %block_id,
    {fields::STATUS} = "error",
    "operation failed"
);
```

**Formatters:**
- `%` = Display (block IDs, hashes, addresses)
- `?` = Debug (structs, enums)
- No formatter = primitives (numbers, booleans)

**Log levels:**
- `error!()` = Requires immediate attention
- `warn!()` = Unexpected but recoverable
- `info!()` = Normal operations (default production)
- `debug!()` = Detailed debugging (dev only)
- `trace!()` = Extremely verbose (almost never used)

---

## Foundations: Structured Logging vs Printf-Style Logging

Before diving into the rules, you need to understand the fundamental difference between structured logging and printf-style logging.

### What is Structured Logging?

**Printf-style (DON'T DO THIS):**
```rust
let block_id = "aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d";
let height = 100;

info!("Processing block {} at height {}", block_id, height);
```

**Output (text only):**
```
2025-12-04T07:13:37.782Z INFO Processing block aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d at height 100
```

**Problem:** This is just a string. You can't:
- Filter logs for a specific block ID
- Query all logs at height 100
- Parse the block ID programmatically
- Build dashboards or metrics
- Search in log aggregation systems (Loki, Datadog, etc.)

---

**Structured logging (DO THIS):**
```rust
use strata_common::fields;

let block_id = "aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d";
let height = 100;

info!(
    {fields::L1_BLOCK} = %block_id,
    {fields::L1_HEIGHT} = height,
    "processing block"
);
```

**Output (human-readable format):**
```
2025-12-04T07:13:37.782Z INFO l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d l1_height=100 processing block
```

**Output (JSON format - enabled via logger config):**
```json
{
  "timestamp": "2025-12-04T07:13:37.782Z",
  "level": "INFO",
  "message": "processing block",
  "fields": {
    "l1_block": "aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d",
    "l1_height": 100
  }
}
```

**Now you can:**
```bash
# Filter all logs for this specific block
grep 'l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d' logs

# In Loki/Grafana
{l1_block="aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d"}

# In Datadog
l1_block:aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d

# Query all blocks at height 100
{l1_height="100"}

# Build metrics
count(rate(logs{l1_height=~".+"}[5m])) by (l1_height)
```

**The difference:** Fields are **structured data**, not just text.

---

### Understanding the `%` and `?` Formatters

**Three ways to log a field:**

```rust
let block_id = BlockId::from_hex("aa026ef...");

// 1. Display format (%)
info!({fields::L1_BLOCK} = %block_id, "log");
// Output: l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d

// 2. Debug format (?)
info!({fields::L1_BLOCK} = ?block_id, "log");
// Output: l1_block=BlockId(0xaa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d)

// 3. Raw value (no formatter - only for primitives)
info!({fields::L1_HEIGHT} = height, "log");
// Output: l1_height=100
```

**When to use each:**

- **`%` (Display):** Use for types implementing `Display` - user-facing representation
  - Block IDs, transaction hashes, addresses
  - Produces clean output: `l1_block=aa026ef...`

- **`?` (Debug):** Use for types implementing `Debug` - developer representation
  - Structs, enums, complex types
  - Shows type info: `status=Ok(Response::Continue)`

- **No formatter:** Use for primitives only
  - Numbers: `height`, `slot`, `count`
  - Booleans: `is_valid`

---

### Log Levels: When to Use Each

```rust
// ERROR - Something failed and requires attention
// Use for: Unrecoverable errors, data corruption, critical failures
error!(
    {fields::ERROR} = %e,
    {fields::L1_BLOCK} = %block_id,
    "failed to process block"
);
// Operator needs to investigate immediately

// WARN - Something unexpected but system continues
// Use for: Retryable errors, degraded performance, unusual conditions
warn!(
    {fields::RETRY_COUNT} = retry_count,
    "RPC call failed, retrying"
);
// Might need attention if it happens repeatedly

// INFO - Normal operation, significant events
// Use for: Block processed, checkpoint verified, service started
info!(
    {fields::L1_BLOCK} = %block_id,
    {fields::L1_HEIGHT} = height,
    "processed block"
);
// Default production log level - shows system progress

// DEBUG - Detailed information for debugging
// Use for: Internal state changes, loop iterations, calculations
debug!(
    anchor_block = %anchor_id,
    "found anchor candidate"
);
// Only enabled when debugging (RUST_LOG=debug)

// TRACE - Extremely verbose, every detail
// Use for: Function entry/exit, every step of an algorithm
trace!("entering process_block function");
// Almost never used - only for deep debugging
```

**Production configuration:**
- Default: `INFO` and above (INFO, WARN, ERROR)
- Debugging: `DEBUG` or `TRACE` as needed
- Never commit code that requires `DEBUG`/`TRACE` for normal operation

---

### Why This Matters: Real Example

**Bad (printf-style):**
```rust
info!("ASM processed block {} at height {}", block_id, height);
info!("CSM received {} logs for block {}", logs.len(), block_id);
info!("Checkpoint verified for block {}", block_id);
```

**Output:**
```
07:13:37.782 INFO ASM processed block aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d at height 100
07:13:37.801 INFO CSM received 5 logs for block aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d
07:13:37.923 INFO Checkpoint verified for block aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d
```

**To find all logs for this block:** You have to grep a 64-character hex string. If it's abbreviated anywhere, you miss logs.

---

**Good (structured):**
```rust
// ASM
info!(
    {fields::COMPONENT} = "asm_worker",
    {fields::L1_BLOCK} = %block_id,
    {fields::L1_HEIGHT} = height,
    "processed block"
);

// CSM
info!(
    {fields::COMPONENT} = "csm_worker",
    {fields::L1_BLOCK} = %block_id,
    log_count = logs.len(),
    "received logs"
);

// Checkpoint
info!(
    {fields::COMPONENT} = "checkpoint_worker",
    {fields::L1_BLOCK} = %block_id,
    "verified checkpoint"
);
```

**Output (human-readable):**
```
07:13:37.782 INFO component=asm_worker l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d l1_height=100 processed block
07:13:37.801 INFO component=csm_worker l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d log_count=5 received logs
07:13:37.923 INFO component=checkpoint_worker l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d verified checkpoint
```

**Now you can:**
```bash
# All logs for this block (across all components)
grep 'l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d' logs

# All ASM worker logs
grep 'component=asm_worker' logs

# All checkpoint verifications
grep 'component=checkpoint_worker' logs | grep 'verified checkpoint'
```

**Output (JSON format):**
```json
{"timestamp":"2025-12-04T07:13:37.782Z","level":"INFO","message":"processed block","fields":{"component":"asm_worker","l1_block":"aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d","l1_height":100}}
{"timestamp":"2025-12-04T07:13:37.801Z","level":"INFO","message":"received logs","fields":{"component":"csm_worker","l1_block":"aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d","log_count":5}}
{"timestamp":"2025-12-04T07:13:37.923Z","level":"INFO","message":"verified checkpoint","fields":{"component":"checkpoint_worker","l1_block":"aa026ef3355b2cd154356a98bebfa700fe093bc1a0d2610005591422d"}}
```

**Now you can query in Loki:**
```logql
# All logs for this block
{l1_block="aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d"}

# All ASM worker logs
{component="asm_worker"}

# Checkpoint verifications that took > 1 second
{component="checkpoint_worker"} | json | duration_ms > 1000
```

**This is why structured logging matters.**

---

### Common Mistakes

**❌ Mistake 1: Formatting values into the message**
```rust
info!("Processing block {}", block_id);  // ❌ Can't query block_id
```

**✅ Fix:**
```rust
info!({fields::L1_BLOCK} = %block_id, "processing block");  // ✅ Queryable field
```

---

**❌ Mistake 2: Using both formatted message AND fields**
```rust
// ❌ Redundant - block_id appears twice
info!({fields::L1_BLOCK} = %block_id, "processing block {}", block_id);
```

**✅ Fix:**
```rust
// ✅ Fields are separate, message is simple
info!({fields::L1_BLOCK} = %block_id, "processing block");
```

---

**❌ Mistake 3: Not using field constants**
```rust
info!(component = "asm_worker", "log");  // ❌ Typo won't be caught
info!(component = "asm_worker", "log");   // ❌ Compiles fine, wrong field!
```

**✅ Fix:**
```rust
use strata_common::fields;
info!({fields::COMPONENT} = "asm_worker", "log");  // ✅ Compile-time checked
```

---

**❌ Mistake 4: Multiple values in one field**
```rust
// ❌ Can't query by height
info!(block_info = format!("{}@{}", block_id, height), "processing");
```

**✅ Fix:**
```rust
// ✅ Separate queryable fields
info!(
    {fields::L1_BLOCK} = %block_id,
    {fields::L1_HEIGHT} = height,
    "processing"
);
```

---

## Core Principles

### 1. Every Component Operation MUST Use Spans

**Rule:** Any function that does significant work MUST create a span.

**What counts as "significant work":**
- Processing blocks/transactions
- Network requests
- Database operations
- State transitions
- Cross-component calls
- Operations that can fail or be slow

**Why:** Spans provide automatic context propagation, duration tracking, and correlation without manual work.

---

#### Real Example: ASM Worker Block Processing

**Current code (crates/asm/worker/src/service.rs:37-104):**

```rust
fn process_input(
    state: &mut AsmWorkerServiceState<W>,
    incoming_block: &L1BlockCommitment,
) -> anyhow::Result<Response> {
    let genesis_height = state.params.rollup().genesis_l1_view.height();
    let height = incoming_block.height();

    if height < genesis_height {
        warn!(%height, "ignoring unexpected L1 block before genesis");  // ❌ No context
        return Ok(Response::Continue);
    }

    // Traverse back to find pivot anchor
    let mut skipped_blocks = vec![];
    let mut pivot_block = *incoming_block;
    let mut pivot_anchor = ctx.get_anchor_state(&pivot_block);

    while pivot_anchor.is_err() && pivot_block.height() >= genesis_height {
        let block = ctx.get_l1_block(pivot_block.blkid())?;
        let parent_height = pivot_block.height().to_consensus_u32() - 1;
        // ... find parent block ...
        skipped_blocks.push((block, pivot_block));
        pivot_anchor = ctx.get_anchor_state(&parent_block_id);
        pivot_block = parent_block_id;
    }

    if pivot_block.height() < genesis_height {
        warn!("ASM hasn't found pivot anchor state at genesis.");  // ❌ No context
        return Ok(Response::ShouldExit);
    }

    info!(%pivot_block, "ASM found pivot anchor state");  // ❌ No component tag
    state.update_anchor_state(pivot_anchor.unwrap(), pivot_block);

    // Process chain of unprocessed blocks
    for (block, block_id) in skipped_blocks.iter().rev() {
        info!(%block_id, "ASM transition attempt");  // ❌ Repeated manually
        match state.transition(block) {
            Ok(asm_stf_out) => {
                let new_state = AsmState::from_output(asm_stf_out);
                state.context.store_anchor_state(block_id, &new_state)?;
                state.update_anchor_state(new_state, *block_id);
            }
            Err(e) => {
                error!(%e, "ASM transition error");  // ❌ Missing block context
                return Ok(Response::ShouldExit);
            }
        }
        info!(%block_id, "ASM transition success");  // ❌ Repeated manually
    }

    Ok(Response::Continue)
}
```

**Problems:**
1. ❌ No span - every log manually repeats `%block_id`
2. ❌ No `component` field - can't filter ASM logs
3. ❌ No `l1_height` field - can't query by height
4. ❌ Error logs missing block context
5. ❌ No duration tracking - can't see how long processing takes
6. ❌ Can't correlate these logs with CSM worker logs for same block

---

**Fixed with span:**

```rust
use tracing::{info_span, Instrument};
use strata_common::fields;

fn process_input(
    state: &mut AsmWorkerServiceState<W>,
    incoming_block: &L1BlockCommitment,
) -> anyhow::Result<Response> {
    // Create span ONCE with all context
    let span = info_span!(
        "process_l1_block",
        {fields::COMPONENT} = "asm_worker",
        {fields::L1_BLOCK} = %incoming_block.blkid(),
        {fields::L1_HEIGHT} = incoming_block.height(),
    );

    // All work happens inside span
    (|| {
        let genesis_height = state.params.rollup().genesis_l1_view.height();
        let height = incoming_block.height();

        if height < genesis_height {
            warn!("ignoring L1 block before genesis");  // ✅ Inherits component, l1_block, l1_height
            return Ok(Response::Continue);
        }

        // Traverse back to find pivot anchor
        let mut skipped_blocks = vec![];
        let mut pivot_block = *incoming_block;
        let mut pivot_anchor = ctx.get_anchor_state(&pivot_block);

        while pivot_anchor.is_err() && pivot_block.height() >= genesis_height {
            let block = ctx.get_l1_block(pivot_block.blkid())?;
            // ... find parent block ...
            skipped_blocks.push((block, pivot_block));
            pivot_anchor = ctx.get_anchor_state(&parent_block_id);
            pivot_block = parent_block_id;
        }

        if pivot_block.height() < genesis_height {
            warn!("pivot anchor not found at genesis");  // ✅ Inherits all fields
            return Ok(Response::ShouldExit);
        }

        info!(pivot_block = %pivot_block, "found pivot anchor state");  // ✅ Inherits component
        state.update_anchor_state(pivot_anchor.unwrap(), pivot_block);

        // Process chain of unprocessed blocks
        info!(block_count = skipped_blocks.len(), "processing skipped blocks");
        for (i, (block, block_id)) in skipped_blocks.iter().rev().enumerate() {
            // Create nested span for each block transition
            let transition_span = info_span!(
                "asm_transition",
                transition_block = %block_id,
                block_index = i,
            );

            let result = (|| {
                info!("attempting transition");  // ✅ Inherits parent + nested span fields

                match state.transition(block) {
                    Ok(asm_stf_out) => {
                        let new_state = AsmState::from_output(asm_stf_out);
                        state.context.store_anchor_state(block_id, &new_state)?;
                        state.update_anchor_state(new_state, *block_id);
                        info!("transition successful");  // ✅ All context inherited
                        Ok(())
                    }
                    Err(e) => {
                        error!(
                            {fields::ERROR} = %e,
                            {fields::STATUS} = "error",
                            "transition failed"  // ✅ Has component, l1_block, transition_block, error
                        );
                        Err(e)
                    }
                }
            })
            .instrument(transition_span)
            ();

            result?;
        }

        info!("L1 block processing complete");  // ✅ Inherits all fields
        Ok(Response::Continue)
    })()
    .instrument(span)  // Attach span to closure
}
```

**What you get:**

**Human-readable logs:**
```
INFO  process_l1_block{component="asm_worker" l1_block=aa026ef... l1_height=100}: new
INFO  process_l1_block{component="asm_worker" l1_block=aa026ef... l1_height=100}: found pivot anchor state pivot_block=aa026ef...
INFO  process_l1_block{component="asm_worker" l1_block=aa026ef... l1_height=100}: processing skipped blocks block_count=3
INFO  process_l1_block{component="asm_worker" l1_block=aa026ef... l1_height=100}:asm_transition{transition_block=bb137fa... block_index=0}: attempting transition
INFO  process_l1_block{component="asm_worker" l1_block=aa026ef... l1_height=100}:asm_transition{transition_block=bb137fa... block_index=0}: transition successful
INFO  process_l1_block{component="asm_worker" l1_block=aa026ef... l1_height=100}:asm_transition{transition_block=bb137fa... block_index=0}: close time.busy=89ms
INFO  process_l1_block{component="asm_worker" l1_block=aa026ef... l1_height=100}: L1 block processing complete
INFO  process_l1_block{component="asm_worker" l1_block=aa026ef... l1_height=100}: close time.busy=234ms
```

**Now you can:**
```bash
# All logs for this specific L1 block (across ALL components)
grep 'l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d' logs

# All ASM worker activity
grep 'component=asm_worker' logs

# All L1 blocks at height 100
grep 'l1_height=100' logs

# All failed transitions
grep 'asm_transition' logs | grep 'status=error'

# See how long each block took to process
grep 'process_l1_block.*close time.busy' logs
```

**Benefits:**
1. ✅ Write context ONCE (in span), inherit everywhere
2. ✅ Automatic duration tracking (`time.busy=234ms`)
3. ✅ All logs have `component`, `l1_block`, `l1_height` automatically
4. ✅ Nested spans show which transition failed
5. ✅ Error logs have full context
6. ✅ Can correlate with CSM logs using same `l1_block` field
7. ✅ OpenTelemetry captures this as distributed trace automatically

---

#### General Pattern

**Forbidden:**
```rust
// ❌ NO SPAN - logs are disconnected
fn process_block(block: &Block) -> Result<()> {
    info!(component = "asm_worker", l1_block = %block.id, "started");  // Manual fields
    do_work()?;
    info!(component = "asm_worker", l1_block = %block.id, "completed");  // Repeated fields
    Ok(())
}
```

**Mandatory:**
```rust
// ✅ SPAN - context set once, inherited everywhere
fn process_block(block: &Block) -> Result<()> {
    let span = info_span!(
        "process_block",
        {fields::COMPONENT} = "asm_worker",
        {fields::L1_BLOCK} = %block.id,
        {fields::L1_HEIGHT} = block.height,
    );

    (|| {
        info!("started processing");  // Inherits all span fields automatically
        do_work()?;
        info!("completed processing");  // Inherits all span fields automatically
        Ok(())
    })()
    .instrument(span)
}
```

---

### 2. Every Span MUST Have a Component Tag

**Rule:** All spans MUST include `{fields::COMPONENT} = "component_name"`.

**Why:** Component tags allow filtering by logical system, which survives refactoring.

**Component names:** See `COMPONENT-NAMES.md` for the canonical list.

**Mandatory:**
```rust
let span = info_span!(
    "verify_checkpoint",
    {fields::COMPONENT} = "checkpoint_worker",  // ✅ REQUIRED
    {fields::CHECKPOINT_IDX} = idx,
);
```

**Forbidden:**
```rust
let span = info_span!("verify_checkpoint", checkpoint_idx = idx);  // ❌ Missing component
```

---

### 3. Use Natural Identifiers for Correlation

**Rule:** Every span processing an entity MUST include that entity's natural identifier.

**Natural identifiers:**
- Block processing → `l1_block` or `l2_block` (full ID)
- Transaction processing → `tx_hash` (full hash)
- Epoch operations → `epoch` (number)
- Checkpoint operations → `checkpoint_idx` (number)
- Slot operations → `l2_slot` (number)

**Why:** Natural IDs let you trace a single entity through the entire system without generating request IDs.

**Mandatory:**
```rust
// Processing L1 block - use block ID
let span = info_span!(
    "process_l1_block",
    {fields::COMPONENT} = "asm_worker",
    {fields::L1_BLOCK} = %block.id,      // ✅ Full block ID
    {fields::L1_HEIGHT} = block.height,
);

// Processing checkpoint - use checkpoint index
let span = info_span!(
    "verify_checkpoint",
    {fields::COMPONENT} = "checkpoint_worker",
    {fields::CHECKPOINT_IDX} = checkpoint.index,  // ✅ Natural ID
);
```

**Forbidden:**
```rust
let req_id = Uuid::new_v4();  // ❌ Don't generate UUIDs
let span = info_span!("process_block", req_id = %req_id);  // ❌ Use natural IDs instead
```

---

### 4. NEVER Abbreviate IDs in Logs

**Rule:** Block IDs, transaction hashes, and other identifiers MUST be logged in full. NO abbreviation, NO truncation.

**Why:** Abbreviated IDs break grep. You cannot find logs for `aa026ef3...91422d` by searching.

**Mandatory:**
```rust
info!({fields::L1_BLOCK} = %block_id, "processed block");
// Output: l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d

info!({fields::TX_HASH} = %tx_hash, "submitted transaction");
// Output: tx_hash=347e16b7626c8f19e1b6cdb5f5...  (full hash)
```

**Forbidden:**
```rust
info!(blkid = %format!("{}..{}", &id[..6], &id[id.len()-6..]), "processed");  // ❌ NEVER
// Output: blkid=aa026e..91422d  ❌ CAN'T GREP THIS
```

**If IDs are too long for display:**
- Full ID in structured field: `l1_block = %full_id`
- Short version in message text: `"processed block {}..{}", &id[..8], &id[id.len()-8..]`
- Consumers can choose what to display, but structured field MUST be full

---

### 5. Use Field Name Constants, Not Raw Strings

**Rule:** Import `strata_common::fields` and use constants for field names.

**Why:**
- Compile-time checking (typos become errors)
- IDE autocomplete
- Consistent naming across codebase

**Mandatory:**
```rust
use strata_common::fields;

info!(
    {fields::COMPONENT} = "asm_worker",     // ✅ Constant
    {fields::L1_BLOCK} = %block_id,         // ✅ Constant
    {fields::STATUS} = "success",           // ✅ Constant
    "operation completed"
);
```

**Forbidden:**
```rust
info!(component = "asm_worker", l1_block = %block_id, "completed");  // ❌ Raw strings
info!(component = "asm_worker", "completed");  // ❌ Typo, compiles fine!
```

**See:** `crates/common/src/fields.rs` for all available constants.

---

### 6. Hot Loops MUST Use Rate Limiting

**Rule:** Loops that run more than 100 times per second MUST rate-limit their logs.

**Why:** Hot loops create log spam that makes debugging impossible and impacts performance.

**Mandatory:**
```rust
for (i, item) in items.iter().enumerate() {
    process_item(item)?;

    // Log every 100th iteration
    if i % 100 == 0 {
        info!(processed = i, total = items.len(), "processing items");
    }
}

// Or log every N seconds
let mut last_log = Instant::now();
for item in items {
    process_item(item)?;

    if last_log.elapsed() > Duration::from_secs(5) {
        info!("still processing items");
        last_log = Instant::now();
    }
}
```

**Forbidden:**
```rust
for item in items {
    process_item(item)?;
    info!("processed item");  // ❌ Logs 10,000 times
}
```

---

### 7. Errors MUST Include Context

**Rule:** When logging errors, include ALL relevant context fields.

**Why:** Error logs without context are useless. We need to know what was being processed when it failed.

**Mandatory:**
```rust
match verify_block(block) {
    Ok(_) => info!("block verified"),
    Err(e) => {
        error!(
            {fields::ERROR} = %e,
            {fields::L1_BLOCK} = %block.id,
            {fields::L1_HEIGHT} = block.height,
            {fields::STATUS} = "error",
            "block verification failed"
        );
    }
}

// Even better - span fields are inherited automatically:
let span = info_span!(
    "verify_block",
    {fields::COMPONENT} = "asm_worker",
    {fields::L1_BLOCK} = %block.id,
    {fields::L1_HEIGHT} = block.height,
);

async move {
    verify_block(block).await?;  // Errors automatically include span context
}
.instrument(span)
.await
```

**Forbidden:**
```rust
match verify_block(block) {
    Err(e) => error!("verification failed: {}", e),  // ❌ No context! Which block?
}
```

---

### 8. Cross-Component Calls MUST Propagate Context

**Rule:** When calling another component, pass correlation fields or ensure OpenTelemetry context propagates.

**Why:** Cross-component operations need to be traceable end-to-end.

**Mandatory:**

**Option A: Pass natural ID in message:**
```rust
// ASM worker sends block ID to CSM
let status = AsmWorkerStatus {
    block_id: block.id.clone(),  // ✅ Pass natural ID
    logs: vec![...],
};
csm_handle.send(status).await?;

// CSM creates span with same block ID
let span = info_span!(
    "process_csm_logs",
    {fields::COMPONENT} = "csm_worker",
    {fields::L1_BLOCK} = %status.block_id,  // ✅ Same ID for correlation
);
```

**Option B: OpenTelemetry automatic propagation:**
```rust
// Just use spans - OpenTelemetry propagates trace context automatically
let span = info_span!("asm_process_block", {fields::L1_BLOCK} = %block.id);
async move {
    // Any async calls here inherit the trace context
    rpc_client.call_method().await?;  // ✅ Trace ID propagated
}
.instrument(span)
.await
```

**Forbidden:**
```rust
// ASM worker
info!("processing block");

// CSM worker (different process)
info!("processing logs");  // ❌ No way to correlate these
```

---

### 9. Performance-Critical Paths MUST Be Instrumented

**Rule:** Any code path that affects block processing latency MUST have spans for timing.

**Why:** We need to identify bottlenecks and track performance trends.

**Mandatory:**
```rust
async fn process_block(block: &Block) -> Result<()> {
    let span = info_span!(
        "process_block",
        {fields::COMPONENT} = "asm_worker",
        {fields::L1_BLOCK} = %block.id,
    );

    async move {
        // Each major step gets its own span
        verify_block(block).await?;     // Has own span inside
        update_state(block).await?;     // Has own span inside
        notify_observers(block).await?; // Has own span inside
        Ok(())
    }
    .instrument(span)
    .await
}
```

When logger is configured with `PerformanceMode::Compact`, you'll see:
```
INFO  verify_block: close time.busy=45ms
INFO  update_state: close time.busy=189ms  ← Bottleneck found!
INFO  notify_observers: close time.busy=12ms
```

---

### 10. RPC Handlers MUST Include Method Name

**Rule:** Every RPC handler MUST log the method name using `{fields::RPC_METHOD}`.

**Why:** RPC logs need to be filterable by method for debugging client issues.

**Mandatory:**
```rust
async fn handle_rpc_call(method: &str, params: Value) -> Result<Value> {
    let span = info_span!(
        "rpc_call",
        {fields::COMPONENT} = "rpc_server",
        {fields::RPC_METHOD} = method,  // ✅ REQUIRED
    );

    async move {
        info!("handling RPC call");
        let result = dispatch(method, params).await?;
        info!("RPC call completed");
        Ok(result)
    }
    .instrument(span)
    .await
}
```

---

## Quick Checklist for Code Review

Before submitting a PR, verify:

- [ ] **Every significant function has a span**
- [ ] **Every span has `{fields::COMPONENT}`**
- [ ] **Every span processing an entity has its natural ID** (`l1_block`, `l2_block`, `tx_hash`, etc.)
- [ ] **All IDs are logged in full, never abbreviated**
- [ ] **Using `strata_common::fields` constants, not raw strings**
- [ ] **Hot loops (>100/sec) have rate limiting**
- [ ] **Errors include all relevant context**
- [ ] **Cross-component calls propagate correlation context**
- [ ] **Performance-critical paths are instrumented**
- [ ] **RPC handlers include `{fields::RPC_METHOD}`**

---

## Examples: Good vs Bad

### Example 1: Block Processing

**❌ Bad:**
```rust
fn process_input(state: &mut State, block: &L1BlockCommitment) -> Result<Response> {
    info!("processing block");  // No context

    let anchor = find_anchor(state, block)?;
    info!("found anchor");  // No context

    state.transition(block)?;
    info!("transition done");  // No context

    Ok(Response::Continue)
}
```

**✅ Good:**
```rust
use strata_common::fields;

fn process_input(state: &mut State, block: &L1BlockCommitment) -> Result<Response> {
    let span = info_span!(
        "process_l1_block",
        {fields::COMPONENT} = "asm_worker",
        {fields::L1_BLOCK} = %block.block_id,
        {fields::L1_HEIGHT} = block.height,
    );

    async move {
        info!("processing L1 block");

        let anchor = find_anchor(state, block).await?;
        info!(pivot_block = %anchor.block_id, "found pivot anchor");

        state.transition(block)?;
        info!("transition complete");

        Ok(Response::Continue)
    }
    .instrument(span)
    .await
}
```

### Example 2: Error Handling

**❌ Bad:**
```rust
match verify_checkpoint(checkpoint) {
    Ok(_) => info!("verified"),
    Err(e) => error!("verification failed: {}", e),  // No context!
}
```

**✅ Good:**
```rust
let span = info_span!(
    "verify_checkpoint",
    {fields::COMPONENT} = "checkpoint_worker",
    {fields::CHECKPOINT_IDX} = checkpoint.index,
    {fields::EPOCH} = checkpoint.epoch,
);

match verify_checkpoint(checkpoint).instrument(span).await {
    Ok(_) => info!("checkpoint verified"),
    Err(e) => {
        error!(
            {fields::ERROR} = %e,
            {fields::STATUS} = "error",
            "checkpoint verification failed"
        );
        // Span fields (component, checkpoint_idx, epoch) are inherited automatically
    }
}
```

### Example 3: Hot Loop

**❌ Bad:**
```rust
for tx in transactions {
    process_transaction(tx)?;
    info!("processed tx");  // Logs 10,000 times!
}
```

**✅ Good:**
```rust
for (i, tx) in transactions.iter().enumerate() {
    process_transaction(tx)?;

    if i % 100 == 0 || i == transactions.len() - 1 {
        info!(
            processed = i + 1,
            total = transactions.len(),
            "processing transactions"
        );
    }
}
```

### Example 4: Cross-Component Correlation

**❌ Bad:**
```rust
// In ASM worker
info!("found pivot anchor");

// In CSM worker (different process)
info!("processing logs");  // No way to correlate!
```

**✅ Good:**
```rust
// In ASM worker
let span = info_span!(
    "process_l1_block",
    {fields::COMPONENT} = "asm_worker",
    {fields::L1_BLOCK} = %block.id,  // Natural ID
);

// In CSM worker
let span = info_span!(
    "process_asm_status",
    {fields::COMPONENT} = "csm_worker",
    {fields::L1_BLOCK} = %status.block_id,  // Same ID - correlated!
);

// Now you can grep l1_block=<id> and see both ASM and CSM logs
```

---

## Why These Rules Matter for Mainnet

**Without these guidelines:**
- "Why is block processing slow?" → Can't tell, no timing
- "Where did this transaction fail?" → Can't trace across components
- "Is checkpoint verification the bottleneck?" → No visibility
- "Show me all logs for this block" → Can't grep, IDs are abbreviated
- "What was the system doing when it crashed?" → Log soup, no context

**With these guidelines:**
- `grep "l1_block=<id>"` → See entire block journey across all components
- `grep "component=asm_worker"` → See all ASM activity, no matter which file
- `grep "rpc_method=eth_getBlockByNumber"` → Debug specific RPC issues
- Performance dashboard shows duration trends → Catch regressions before mainnet
- Error logs include full context → Debug in minutes, not hours

**Cost:** ~5 extra lines per function (one span)
**Benefit:** System that can be debugged in production

---

## Exceptions

**When can you skip these rules?**

1. **Trivial utility functions** (string formatting, simple calculations) - no span needed
2. **Inner helper functions** called within an existing span - no additional span needed
3. **Test code** - not required (but still helpful)

**Everything else MUST follow these guidelines.**

---

## References

- **Field constants:** `crates/common/src/fields.rs`
- **Component names:** `COMPONENT-NAMES.md`
- **Detailed examples:** `standards-to-follow.md`
- **Getting started:** `getting-started.md`
- **Action items:** `action-items.md`

---

**Last updated:** 2025-12-04
**Status:** MANDATORY for all new code
