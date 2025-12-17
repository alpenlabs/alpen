# Tracing Quick Start - Keep It Simple

## TL;DR - One Span, Natural IDs

```rust
use tracing::{info_span, info, Instrument};

async fn process_l1_block(block: L1BlockCommitment) -> Result<()> {
    let span = info_span!(
        "process_l1_block",
        component = "asm_worker",
        l1_block = %block.block_id,    // ← The block ID IS the identifier
        l1_height = block.height,
    );

    async move {
        info!("processing L1 block");
        do_work(&block).await?;
        info!("processing complete");
        Ok(())
    }
    .instrument(span)
    .await
}
```

That's it. No frameworks, no UUID generators, no complexity.

---

## The Philosophy

### Use What You Already Have

**You don't need request IDs.** You already have natural unique identifiers:

| Operation | Natural Identifier |
|-----------|-------------------|
| L1 block processing | `l1_block` (the block ID itself) |
| L2 block processing | `l2_block` |
| Checkpoint processing | `epoch` or `checkpoint_idx` |
| Fork choice | `l1_height` |
| RPC calls | `rpc_method` + arguments |

**Grep by natural ID:**
```bash
# See everything for this specific L1 block
$ grep "l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d" logs

# See all L1 height 100 processing
$ grep "l1_height=100" logs

# See all checkpoint processing for epoch 42
$ grep "epoch=42" logs
```

---

## What To Do

### 1. Create ONE Span at Entry Point

```rust
let span = info_span!(
    "operation_name",
    component = "your_component",  // Semantic grouping
    // Add natural identifiers (block ID, height, epoch, etc.)
);
```

### 2. Wrap Your Work

```rust
async move {
    // All logs here automatically get component + your fields
    info!("started");
    work().await?;
    info!("done");
}
.instrument(span)
.await
```

### 3. Child Operations Inherit Context

```rust
async move {
    info!("parent operation");

    // Child span inherits component, l1_block, l1_height from parent
    let child_span = info_span!("child_operation");
    async move {
        info!("child work");  // Has all parent fields + child span name
    }
    .instrument(child_span)
    .await?;

    info!("parent complete");
}
.instrument(span)
.await
```

---

## Field Naming Conventions

### Core Identity (Always Include)
- `component` - Semantic component name (see table below)
- `l1_block` - Full L1 block ID (32-byte hex, NEVER abbreviated)
- `l1_height` - L1 block height
- `l2_block` - Full L2 block ID (32-byte hex, NEVER abbreviated)
- `l2_slot` - L2 slot number
- `epoch` - Epoch number
- `checkpoint_idx` - Checkpoint index
- `tx_hash` - Full transaction hash (NEVER abbreviated)

### Status/Outcome
- `status` - "success", "error", "timeout"
- `duration_ms` - Operation duration (u64)
- `error` - Error message (on error events only)

### Operational
- `operation` - Specific operation within component
- `rpc_method` - RPC method name
- `peer_id` - Peer identifier

---

## Component Names

Use consistent component names (lowercase, underscore-separated):

| Code Location | Component Name |
|--------------|----------------|
| `crates/asm/worker/` | `asm_worker` |
| `crates/csm-worker/` | `csm_worker` |
| `crates/btcio/reader/` | `l1_reader` |
| `crates/consensus-logic/fork_choice*` | `fork_choice` |
| `crates/consensus-logic/chain_worker*` | `chain_worker` |
| `crates/consensus-logic/sync_manager*` | `sync_manager` |
| `crates/sequencer/block_template/` | `block_template` |
| `crates/sequencer/checkpoint/` | `checkpoint_worker` |
| `crates/eectl/` | `evm_engine` |
| `crates/chaintsn/` | `state_transition` |
| `crates/storage/` | `storage` |
| RPC client code | `rpc_client` |
| RPC server code | `rpc_server` |

**Multiple modules, same logical component?** Use the same component name.

Example: `fork_choice_manager.rs`, `unfinalized_tracker.rs`, `tip_update.rs` all use `component = "fork_choice"`.

---

## Real Examples

### Example 1: L1 Block Processing (ASM Worker)

```rust
// crates/asm/worker/src/service.rs
use tracing::{info_span, info, error, warn, Instrument};

impl SyncService for AsmWorkerService {
    fn process_input(
        state: &mut AsmWorkerServiceState,
        block: &L1BlockCommitment,
    ) -> anyhow::Result<Response> {
        let span = info_span!(
            "process_l1_block",
            component = "asm_worker",
            l1_block = %block.block_id,
            l1_height = block.height,
        );

        async move {
            info!("ASM processing L1 block");

            let anchor = match find_pivot_anchor(state, block).await {
                Ok(a) => {
                    info!(pivot_block = %a.block_id, "found pivot anchor");
                    a
                }
                Err(e) => {
                    error!(error = %e, "failed to find anchor");
                    return Err(e);
                }
            };

            state.transition(block)?;
            info!("ASM transition complete");

            Ok(Response::Continue)
        }
        .instrument(span)
        .await
    }
}
```

**Logs:**
```
INFO component=asm_worker l1_block=aa026ef...422d l1_height=100: ASM processing L1 block
INFO component=asm_worker l1_block=aa026ef...422d l1_height=100 pivot_block=def456: found pivot anchor
INFO component=asm_worker l1_block=aa026ef...422d l1_height=100: ASM transition complete
```

**Grep:**
```bash
# See everything for this block
$ grep "l1_block=aa026ef3355b2cd154356a98bebfa700fe093bc1d50cb71a0d2610005591422d" logs

# See all ASM worker activity
$ grep "component=asm_worker" logs

# See all height 100 processing
$ grep "l1_height=100" logs
```

---

### Example 2: Checkpoint Processing (CSM Worker)

```rust
// crates/csm-worker/src/service.rs
impl SyncService for CsmWorkerService {
    fn process_input(
        state: &mut CsmWorkerState,
        asm_status: &AsmWorkerStatus,
    ) -> anyhow::Result<Response> {
        let asm_block = asm_status.cur_block?;

        let span = info_span!(
            "process_checkpoints",
            component = "csm_worker",
            l1_block = %asm_block,
        );

        async move {
            info!("CSM processing ASM logs");

            for log in asm_status.logs() {
                if let Some(checkpoint) = extract_checkpoint(log) {
                    info!(epoch = checkpoint.epoch, "found checkpoint");
                    update_client_state(state, checkpoint)?;
                }
            }

            info!("CSM processing complete");
            Ok(Response::Continue)
        }
        .instrument(span)
        .await
    }
}
```

**Cross-Crate Correlation:**
If ASM and CSM both log the same `l1_block`, grep shows the complete flow:
```
INFO component=asm_worker l1_block=aa026ef...422d: ASM processing
INFO component=csm_worker l1_block=aa026ef...422d: CSM processing
INFO component=csm_worker l1_block=aa026ef...422d epoch=42: found checkpoint
```

---

### Example 3: Fork Choice (Multiple Modules, Same Component)

```rust
// crates/consensus-logic/src/fork_choice_manager.rs
pub fn select_tip(&self, candidates: Vec<BlockId>) -> Result<BlockId> {
    let span = info_span!(
        "select_chain_tip",
        component = "fork_choice",
        candidate_count = candidates.len(),
    );

    let _guard = span.enter();

    info!("evaluating fork choice candidates");

    let tips = self.unfinalized_tracker.get_tips()?;
    info!(tip_count = tips.len(), "found competing tips");

    let selected = self.choose_best(tips)?;
    info!(selected_tip = %selected, "chain tip selected");

    Ok(selected)
}

// crates/consensus-logic/src/unfinalized_tracker.rs
pub fn get_tips(&self) -> Result<Vec<BlockId>> {
    let span = info_span!(
        "get_fork_tips",
        component = "fork_choice",  // ← Same component!
    );

    let _guard = span.enter();

    info!("scanning block tree for tips");
    let tips = self.tree.find_tips()?;
    info!(found = tips.len(), "tips identified");

    Ok(tips)
}
```

**Both modules use `component = "fork_choice"`**, so:
```bash
$ grep "component=fork_choice" logs
# Shows activity from both fork_choice_manager.rs AND unfinalized_tracker.rs
```

---

### Example 4: Rate Limiting in Hot Loops

```rust
async fn process_batch(blocks: Vec<L1Block>) -> Result<()> {
    let span = info_span!(
        "process_batch",
        component = "l1_reader",
        batch_size = blocks.len(),
    );

    async move {
        let mut processed = 0;

        for block in blocks {
            process_block(&block).await?;
            processed += 1;

            // Only log every 10th block
            if processed % 10 == 0 {
                info!(
                    processed,
                    last_height = block.height,
                    "batch progress"
                );
            }
        }

        // Always log final summary
        info!(total_processed = processed, "batch complete");
        Ok(())
    }
    .instrument(span)
    .await
}
```

**Result:**
- 1000 blocks → ~100 logs (instead of 1000)
- Progress updates every 10 blocks
- Final summary always shown

---

## Common Patterns

### Pattern 1: Simple Operation
```rust
let span = info_span!(
    "operation_name",
    component = "your_component",
    // Add natural IDs
);

async move {
    info!("started");
    work().await?;
    info!("done");
}.instrument(span).await
```

### Pattern 2: With Error Handling
```rust
async move {
    info!("processing started");

    match process().await {
        Ok(result) => {
            info!(status = "success", "processing complete");
            Ok(result)
        }
        Err(e) => {
            error!(error = %e, status = "error", "processing failed");
            Err(e)
        }
    }
}.instrument(span).await
```

### Pattern 3: Manual Timing
```rust
async move {
    let start = std::time::Instant::now();

    work().await?;

    let duration_ms = start.elapsed().as_millis() as u64;
    info!(duration_ms, "operation complete");
}.instrument(span).await
```

### Pattern 4: Multiple Stages
```rust
async move {
    info!("stage 1: preparing");
    let data = prepare().await?;

    info!("stage 2: processing");
    let result = process(data).await?;

    info!("stage 3: finalizing");
    finalize(result).await?;

    info!("all stages complete");
}.instrument(span).await
```

---

## What NOT To Do

### ❌ Don't Generate Random UUIDs
```rust
// BAD
let req_id = uuid::Uuid::new_v4();
let span = info_span!("op", req_id = %req_id, l1_block = %block_id);
// The block ID already uniquely identifies this operation!

// GOOD
let span = info_span!("op", l1_block = %block_id);
```

### ❌ Don't Abbreviate IDs
```rust
// BAD
let short_id = format!("{}..{}", &block_id[..6], &block_id[58..]);
info!(block = %short_id, "processing");
// Can't grep for full ID!

// GOOD
info!(l1_block = %full_block_id, "processing");
```

### ❌ Don't Repeat Fields on Every Log
```rust
// BAD
info!(component = "asm", l1_height = 100, "log 1");
info!(component = "asm", l1_height = 100, "log 2");

// GOOD
let span = info_span!("op", component = "asm", l1_height = 100);
async move {
    info!("log 1");  // Inherits component, l1_height
    info!("log 2");  // Inherits component, l1_height
}.instrument(span).await
```

### ❌ Don't Use Magic Strings
```rust
// BAD
let span = info_span!("op", component = "asm-worker");  // Inconsistent!

// GOOD
let span = info_span!("op", component = "asm_worker");  // Standard name
```

### ❌ Don't Log Every Iteration
```rust
// BAD
for i in 0..10000 {
    debug!("iteration {}", i);  // 10,000 logs!
}

// GOOD
for i in 0..10000 {
    process(i)?;
    if i % 100 == 0 {
        debug!(iteration = i, "progress");  // 100 logs
    }
}
info!("all iterations complete");
```

---

## Enabling Verbose Mode

Current `logging.rs` supports basic configuration. You can extend it:

```rust
// In crates/common/src/logging.rs
use tracing_subscriber::fmt::format::FmtSpan;

pub fn init_with_span_events(config: LoggerConfig) {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)  // Show span lifecycle
        .with_line_number(true)  // Show line numbers
        .with_filter(filt);

    // ... rest of init
}
```

Then in your binary:
```rust
if args.verbose {
    init_with_span_events(config);
} else {
    init(config);
}
```

---

## OpenTelemetry (Already Set Up!)

You already have OpenTelemetry configured in `logging.rs`:
```rust
if let Some(otel_url) = &config.otel_url {
    let otel_sub = tracing_opentelemetry::layer().with_tracer(tt);
    // Automatically propagates trace context across RPC boundaries!
}
```

**To enable:**
```bash
$ export STRATA_OTLP_URL=http://localhost:4317
$ cargo run
```

OpenTelemetry automatically:
- Generates W3C trace IDs
- Propagates them across RPC calls
- Handles distributed tracing

**You don't need custom code for RPC tracing.**

---

## Summary

### Do This:
1. Create ONE span at entry point
2. Use natural identifiers (block ID, epoch, slot)
3. Add `component` field for semantic grouping
4. Use `.instrument(span)` wrapper
5. All logs inherit context automatically

### Don't Do This:
1. Generate random UUIDs (you have natural IDs)
2. Abbreviate block IDs (breaks grep)
3. Repeat fields on every log (use spans)
4. Use inconsistent component names (see table)
5. Log every iteration in hot loops (rate limit)

### What You Get:
```bash
# See everything for one block
$ grep "l1_block=aa026ef..." logs

# See all fork choice activity
$ grep "component=fork_choice" logs

# See all operations at height 100
$ grep "l1_height=100" logs
```

**No frameworks. No complexity. Just tracing spans and natural IDs.**
