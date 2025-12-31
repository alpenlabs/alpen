# Observability Naming Conventions

**Status:** Standard
**Last Updated:** 2025-12-10

## Purpose

Consistent naming enables:
- **Grepability:** Find all logs for a specific entity (block, tx, checkpoint) instantly
- **Metrics aggregation:** Group related events automatically
- **Cross-service correlation:** Same names in all services for same entities
- **Tooling:** Automated analysis and dashboards

## Core Principles

1. **Explicit over implicit:** Name variables to match log field names
2. **Full identifiers:** Never abbreviate hashes (no `..`, no `[..4]`)
3. **Consistent prefixes:** `l1_*` for L1 entities, `l2_*` for L2 entities
4. **Snake_case:** All field names and variable names use snake_case

---

## Entity Naming Standards

### L1 Entities

| Entity | Variable Name | Log Field | Type | Example |
|--------|--------------|-----------|------|---------|
| L1 Block Height | `l1_height` | `l1_height` | `u64` | `100` |
| L1 Block ID/Hash | `l1_block` | `l1_block` | `L1BlockId` | `347e16b7...0eb30` (full) |
| L1 Transaction | `l1_tx` | `l1_tx` | `Txid` | `a1b2c3d4...` (full) |

**Example:**
```rust
// ✅ GOOD
fn process_l1_block(l1_block: &L1BlockId, l1_height: u64) {
    info!(%l1_block, l1_height, "processing L1 block");
}

// ❌ BAD - inconsistent naming
fn process_l1_block(block_id: &L1BlockId, height: u64) {
    info!("processing block {}", block_id);  // No structured fields
}

// ❌ BAD - abbreviated
fn process_l1_block(l1_block: &L1BlockId, l1_height: u64) {
    info!("block {}@{}..{}", l1_height, &l1_block[..4], &l1_block[l1_block.len()-4..]);
}
```

### L2 Entities

| Entity | Variable Name | Log Field | Type | Example |
|--------|--------------|-----------|------|---------|
| L2 Slot | `l2_slot` | `l2_slot` | `u64` | `8` |
| L2 Block ID/Hash | `l2_block` | `l2_block` | `L2BlockId` | `90683b47...6d52` (full) |
| L2 Transaction | `l2_tx` | `l2_tx` | `TxHash` | `deadbeef...` (full) |

**Example:**
```rust
// ✅ GOOD
fn process_l2_block(l2_block: &L2BlockId, l2_slot: u64) {
    info!(%l2_block, l2_slot, "processing L2 block");
}
```

### Checkpoint Entities

| Entity | Variable Name | Log Field | Type | Example |
|--------|--------------|-----------|------|---------|
| Checkpoint Index | `checkpoint_index` | `checkpoint_index` | `u64` | `42` |
| Checkpoint Hash | `checkpoint_hash` | `checkpoint_hash` | `Hash` | `abcd1234...` (full) |

**Example:**
```rust
// ✅ GOOD
fn verify_checkpoint(checkpoint_index: u64, checkpoint_hash: &Hash) {
    info!(checkpoint_index, %checkpoint_hash, "verifying checkpoint");
}
```

### Trace Correlation

| Entity | Variable Name | Log Field | Type | Example |
|--------|--------------|-----------|------|---------|
| Request ID | `req_id` | `req_id` | `String` | `"a1b2c3d4"` (8 chars) |
| Component | `component` | `component` | `&'static str` | `"asm_worker"` |

**Example:**
```rust
// ✅ GOOD
let trace_ctx = TraceContext::from_current_span();
let req_id = trace_ctx.short_id();

info!(%req_id, component = "asm_worker", "processing message");
```

---

## Component Names (Canonical List)

**Service Components:**
- `asm_worker` - ASM (Assignment State Manager) worker
- `csm_worker` - CSM (Checkpoint State Manager) worker
- `chain_worker` - Chain state worker
- `prover_client` - Prover service client
- `prover_service` - Prover service backend

**Infrastructure Components:**
- `btcio` - Bitcoin I/O layer
- `strata_rpc` - RPC server
- `fork_choice` - Fork choice manager
- `block_template` - Block template generation
- `consensus` - Consensus logic

Add new components to this list as needed (update this doc).

---

## Logging Patterns

### Structured Logging (Required)

**Always use structured fields, never string interpolation:**

```rust
// ✅ GOOD - structured fields
info!(
    component = "asm_worker",
    l1_height = block.height(),
    l1_block = %block.id(),
    "processing L1 block"
);

// ❌ BAD - string interpolation
info!("processing block {} at height {}", block.id(), block.height());
```

### Span Fields (Required)

**All spans must have `component` field:**

```rust
// ✅ GOOD
#[instrument(
    skip_all,
    fields(
        component = "asm_worker",
        l1_height = block.height(),
        l1_block = %block.id(),
    )
)]
fn process_block(block: &L1Block) {
    // ...
}

// ❌ BAD - missing component
#[instrument(skip_all)]
fn process_block(block: &L1Block) {
    // ...
}
```

### Error Logging (Required)

**Every error must include: operation, entity IDs, error message, component**

```rust
// ✅ GOOD
match state.transition(block) {
    Ok(output) => output,
    Err(e) => {
        error!(
            error = %e,
            component = "asm_worker",
            l1_height = block.height(),
            l1_block = %block.id(),
            operation = "asm_transition",
            "state transition failed"
        );
        return Err(e);
    }
}

// ❌ BAD - missing context
match state.transition(block) {
    Err(e) => error!("transition failed: {}", e),
    // ...
}
```

---

## Variable Naming Conventions

### Prefer Explicit Names

When variable names match log field names, logging becomes natural:

```rust
// ✅ GOOD - explicit names match log fields
fn process_block(l1_block: &L1BlockId, l1_height: u64) {
    info!(%l1_block, l1_height, "processing");  // Natural!
}

// ❌ BAD - generic names require renaming in logs
fn process_block(id: &L1BlockId, height: u64) {
    info!(l1_block = %id, l1_height = height, "processing");  // Awkward
}
```

### Rename on Boundaries

If a function receives a generic name, rename it immediately:

```rust
// ✅ GOOD - rename at boundary
fn process_commitment(commitment: &L1BlockCommitment) {
    let l1_height = commitment.height();
    let l1_block = commitment.blkid();

    info!(%l1_block, l1_height, "processing commitment");
}

// ❌ BAD - keep generic names
fn process_commitment(commitment: &L1BlockCommitment) {
    info!(
        l1_block = %commitment.blkid(),
        l1_height = commitment.height(),
        "processing"
    );  // Verbose every time
}
```

### Loop Variables

In loops, use full names not abbreviations:

```rust
// ✅ GOOD
for l1_block in blocks {
    info!(%l1_block, "processing block");
}

// ❌ BAD
for blk in blocks {
    info!(l1_block = %blk, "processing block");
}
```

---

## Display Implementations

### Full Identifiers Required

**Block IDs, transaction hashes, and other identifiers must display fully:**

```rust
// Current problematic Display implementations:
// - L1BlockId displays as "height@prefix..suffix"
// - Should display full hash

// Recommended: Create explicit display helpers if needed
impl L1BlockId {
    pub fn to_full_string(&self) -> String {
        format!("{}", self.blkid)  // Full hash
    }

    pub fn to_short_string(&self) -> String {
        format!("{}@{}..{}", self.height, &self.blkid[..4], &self.blkid[self.blkid.len()-4..])
    }
}

// In logs, always use full:
info!(l1_block = %block_id.to_full_string(), "processing");

// Or use Display directly if it shows full hash:
info!(%l1_block, "processing");  // Assumes Display shows full hash
```

### Formatting Directives

- `%field` - Uses Display trait
- `?field` - Uses Debug trait
- `field` - For primitive types (u64, bool, etc.)

```rust
info!(
    %l1_block,           // Display (should show full hash)
    l1_height,           // Primitive u64
    ?complex_struct,     // Debug for complex types
    "message"
);
```

---

## Migration Strategy

### Step 1: Audit Existing Code

Find violations:
```bash
# Find abbreviated identifiers
rg '(\.\.|truncate|@[a-f0-9]{4}\.\.[a-f0-9]{4})' --type rust

# Find unstructured logging
rg 'info!\(".*\{' --type rust

# Find spans without component
rg '#\[instrument' --type rust -A3 | grep -v 'component ='
```

### Step 2: Rename Variables

Priority order:
1. **Hot paths** - ASM worker, CSM worker, fork choice
2. **RPC handlers** - Cross-service boundaries
3. **Background tasks** - Lower priority

Example PR for one file:
```rust
// Before
fn process_input(state: &mut State, incoming_block: &L1BlockCommitment) -> Result<Response> {
    let height = incoming_block.height();
    let block_id = incoming_block.blkid();
    info!("processing block {} at height {}", block_id, height);
}

// After
fn process_input(state: &mut State, incoming_block: &L1BlockCommitment) -> Result<Response> {
    let l1_height = incoming_block.height();
    let l1_block = incoming_block.blkid();
    info!(%l1_block, l1_height, component = "asm_worker", "processing L1 block");
}
```

### Step 3: Update Display Implementations

If types like `L1BlockId` have abbreviated Display, either:
- **Option A:** Change Display to show full hash (breaking change)
- **Option B:** Add `.to_full_string()` method, use explicitly
- **Option C:** Use Debug trait with `?` instead of `%`

### Step 4: Enforce via CI

Add CI check (see Phase 6 in action-items.md):
```yaml
- name: Check abbreviated identifiers
  run: |
    if rg '\.\..*blk|blkid.*\.\.' --type rust crates/; then
      echo "❌ Found abbreviated block IDs"
      exit 1
    fi
```

---

## Compile-Time Enforcement (Optional)

**Question:** Can we enforce naming at compile time without being verbose?

**Answer:** Partially, with type wrappers.

### Option 1: Newtype Wrappers (Recommended)

Create zero-cost wrappers that enforce correct Display:

```rust
// In common crate
#[derive(Copy, Clone)]
pub struct LoggableL1Block<'a>(&'a L1BlockId);

impl Display for LoggableL1Block<'_> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.0.full_hash())  // Always full
    }
}

// Extension trait for convenience
pub trait LoggableExt {
    fn loggable(&self) -> LoggableL1Block<'_>;
}

impl LoggableExt for L1BlockId {
    fn loggable(&self) -> LoggableL1Block<'_> {
        LoggableL1Block(self)
    }
}

// Usage
info!(l1_block = %block_id.loggable(), "processing");
```

**Pros:** Guaranteed full identifiers, zero runtime cost
**Cons:** Slightly more verbose (`.loggable()`)

### Option 2: Enforce via Variable Names (Implicit)

**Simpler:** Just rename variables and rely on code review + CI checks.

```rust
// Good variable names make logging natural
fn process_block(l1_block: &L1BlockId, l1_height: u64) {
    info!(%l1_block, l1_height, "processing");
}
```

**Pros:** Zero boilerplate, clear intent
**Cons:** Not compile-time enforced, relies on discipline

### Recommendation

**Use Option 2 (implicit enforcement)** because:
1. Adding `.loggable()` everywhere is verbose (you mentioned not wanting this)
2. Clear variable names + CI checks catch 95% of issues
3. Can add Option 1 later if needed for specific types

---

## Examples

### Full Example: ASM Worker

```rust
use tracing::*;

#[instrument(
    skip_all,
    fields(
        component = "asm_worker",
        l1_height = incoming_block.height(),
        l1_block = %incoming_block.blkid(),
    )
)]
fn process_input(
    state: &mut AsmWorkerServiceState<W>,
    incoming_block: &L1BlockCommitment,
) -> anyhow::Result<Response> {
    let l1_height = incoming_block.height();
    let l1_block = incoming_block.blkid();

    if l1_height < state.genesis_height() {
        warn!(l1_height, %l1_block, "ignoring pre-genesis block");
        return Ok(Response::Continue);
    }

    // Find pivot
    let (pivot_block, skipped_blocks) = {
        let span = debug_span!("find_pivot", component = "asm_worker");
        let _guard = span.enter();

        find_pivot_anchor(state, incoming_block)?
    };

    info!(
        %pivot_block,
        skipped_count = skipped_blocks.len(),
        "found pivot anchor"
    );

    // Process blocks
    for (block, block_id) in skipped_blocks.iter().rev() {
        let l1_height = block_id.height();
        let l1_block = block_id.blkid();

        let _span = info_span!(
            "asm_transition",
            component = "asm_worker",
            l1_height,
            %l1_block,
        ).entered();

        match state.transition(block) {
            Ok(output) => {
                info!("transition succeeded");
            }
            Err(e) => {
                error!(
                    %e,
                    l1_height,
                    %l1_block,
                    operation = "asm_transition",
                    "transition failed"
                );
                return Ok(Response::ShouldExit);
            }
        }
    }

    Ok(Response::Continue)
}
```

### Full Example: RPC Handler

```rust
use strata_common::tracing_context::{TraceContext, inject_trace_context};

async fn get_blocks_at_idx(
    &self,
    idx: u64,
    trace_ctx: Option<TraceContext>,
) -> RpcResult<Vec<HexBytes32>> {
    let trace_ctx = trace_ctx.unwrap_or_else(TraceContext::new_root);
    let req_id = trace_ctx.short_id().to_string();

    inject_trace_context(&trace_ctx);

    let span = info_span!(
        "rpc_handler",
        component = "strata_rpc",
        rpc_method = "getBlocksAtIdx",
        %req_id,
        idx,
    );

    async move {
        info!(idx, "handling request");

        self.storage
            .l2()
            .get_blocks_at_slot(idx)
            .await
            .map(|blocks| blocks.into_iter().map(HexBytes32).collect())
            .map_err(|e| {
                error!(
                    %e,
                    idx,
                    %req_id,
                    operation = "get_blocks_at_idx",
                    component = "strata_rpc",
                    "storage query failed"
                );
                to_jsonrpsee_error(e)
            })
    }
    .instrument(span)
    .await
}
```

---

## Checklist

Use this for code review:

- [ ] All log statements use structured fields (not string interpolation)
- [ ] All spans have `component` field
- [ ] Variable names match log field names (`l1_height`, `l1_block`, etc.)
- [ ] Block IDs and hashes shown in full (no `..` abbreviation)
- [ ] Errors include: operation, entity IDs, error message, component
- [ ] Component name is from canonical list
- [ ] Cross-service RPC calls propagate `req_id`

---

## References

- OpenTelemetry Semantic Conventions: https://opentelemetry.io/docs/specs/semconv/
- Structured Logging: https://www.honeycomb.io/blog/structured-logging
- Tracing Field Documentation: https://docs.rs/tracing/latest/tracing/field/index.html
