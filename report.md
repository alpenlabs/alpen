# SSZ Zero-Copy Views for Checkpoint Proofs — Status Report

## Goal

Reduce zkVM cycle costs in the checkpoint proof guest (`checkpoint-new`) by using SSZ **view types** (zero-copy `Ref` wrappers over `&[u8]`) instead of fully-deserialized owned types.

Currently, `process_ol_stf` reads raw SSZ bytes from the zkVM and immediately deserializes them into owned Rust structs (`OLState`, `Vec<OLBlock>`, `OLBlockHeader`). This full deserialization copies every byte, allocates heap memory for all nested variable-length fields, and burns proving cycles even for data that is only hashed or partially accessed. The SSZ view types (`OLBlockRef<'a>`, `OLBlockHeaderRef<'a>`, etc.) already exist in ssz-gen output, but cannot yet be used effectively due to missing infrastructure.

---

## What I Have Done So Far (STR-2129, in progress)

The base checkpoint guest code (`process_ol_stf` / `process_ol_stf_core` / `execute_block_batch` in `crates/proof-impl/checkpoint-new/src/statements.rs`) is functional. It reads 4 SSZ-encoded inputs from the zkVM, executes the OL STF across a batch of blocks, verifies structural integrity, and produces a `CheckpointClaim`.

### Current changes on this branch — Rust-level refactoring (NOT zero-copy)

The current changes on `statements.rs` are a **Rust-level optimization**, not SSZ zero-copy work. I replaced `construct_block` (which clones block components and reconstructs a full header for comparison) with `execute_block_inputs` (which takes borrowed references and compares header fields individually).

**What changed:**
- Calling `execute_block_inputs` directly with `&` references (via `BlockExecInput` which holds `&OLTxSegment` and `Option<&OLL1ManifestContainer>`) avoids cloning block components per block
- Verifying header fields individually (state_root, logs_root, body_root, parent_blkid) instead of reconstructing a full header to compare
- Caching `parent_blkid` across loop iterations to avoid redundant re-hashing
- Returning `&OLBlockHeader` (borrowed from input) instead of a cloned header

**What this does NOT do:**
- Does NOT use SSZ Ref/view types
- All inputs are still fully deserialized into owned types: `Vec::<OLBlock>::from_ssz_bytes(...)`, `OLBlockHeader::from_ssz_bytes(...)`, etc.
- All hashing still re-serializes owned structs: `block.header().compute_blkid()` calls `as_ssz_bytes()` internally, then hashes — the original SSZ bytes are thrown away during deserialization and reconstructed from scratch

This is a valid optimization (eliminating clones and redundant hashes), but it stays entirely in the owned-type world. The actual zero-copy SSZ view work — the main goal of STR-2129 — is blocked by missing infrastructure described below.

### ssz-gen already generates Ref types

The ssz-gen codegen **already** produces `Ref` view types for every SSZ container:

- `OLBlockRef<'a>`, `OLBlockHeaderRef<'a>`, `OLBlockBodyRef<'a>`, etc. are generated and re-exported from `strata_ol_chain_types_new`
- These wrap `&'a [u8]` with lazy getter methods — zero allocation on construction
- `DecodeView` trait provides `from_ssz_bytes(&'a [u8]) -> Result<Self, DecodeError>` (validates structure without copying)
- `ToOwnedSsz` trait enables conversion back to owned types when mutation is needed
- `TreeHash` implementations are generated for both owned and view types
- Collection views (`VariableListRef`, `FixedVectorRef`) already have `pub fn as_bytes(&self) -> &'a [u8]`

---

## What's Actually Blocking From Using Ref/View Types

### Blocker 1: Container Ref types lack `as_bytes()` — ssz-gen codegen gap

**The problem**: Generated container Ref types (e.g., `OLBlockHeaderRef<'a>`) have a **private** `bytes: &'a [u8]` field with no public accessor. Collection views (`VariableListRef`, `ListRef`) already expose `as_bytes()`, but the codegen for container structs does not generate this method.

**Why it matters**: The checkpoint guest needs to hash headers and bodies constantly:
- `compute_blkid()` = hash of header's SSZ bytes (called N times per batch)
- `compute_hash_commitment()` = hash of body's SSZ bytes (called N times per batch)

With owned types, these methods call `self.as_ssz_bytes()` which **re-serializes** the already-deserialized struct back to bytes, then hashes. With Ref types + `as_bytes()`, we'd hash the original bytes directly — no re-serialization.

Without `as_bytes()`, there is no way to implement `compute_blkid()` on `OLBlockHeaderRef` because the underlying bytes are inaccessible.

**The fix**: Add one method to the generated impl block in `ssz_codegen/src/types/mod.rs`, function `to_view_getters` (line 1615):

```rust
pub const fn as_bytes(&self) -> &'a [u8] {
    self.bytes
}
```

The pattern already exists for `ListRef` (`ssz/src/view.rs:326`) and `VariableListRef` (`ssz_types/src/view.rs:97`).

### Blocker 2: No compute/helper methods on Ref types in alpen

**The problem**: `OLBlockHeader` (owned) has convenience methods — `compute_blkid()`, `compute_block_commitment()`, `is_terminal()`, `slot()`, `epoch()`, etc. — but `OLBlockHeaderRef` has none of these. Same for `OLBlockBody` vs `OLBlockBodyRef`.

The Ref types only have raw field getter methods (generated by ssz-gen, return `Result<T, DecodeError>`). To use Ref types in `execute_block_batch`, the same domain methods that the owned types have are needed.

**What's needed** (in `crates/ol/chain-types/src/block.rs`, depends on Blocker 1):
```rust
impl<'a> OLBlockHeaderRef<'a> {
    pub fn compute_blkid(&self) -> OLBlockId {
        OLBlockId::from(hash::raw(self.as_bytes()))
    }
    pub fn compute_block_commitment(&self) -> Result<OLBlockCommitment, DecodeError> {
        Ok(OLBlockCommitment::new(self.slot()?, self.compute_blkid()))
    }
    pub fn is_terminal(&self) -> Result<bool, DecodeError> {
        Ok(self.flags()?.is_terminal())
    }
}

impl<'a> OLBlockBodyRef<'a> {
    pub fn compute_hash_commitment(&self) -> Buf32 {
        hash::raw(self.as_bytes())
    }
}
```

### Blocker 3: STF functions only accept owned types

**The problem**: `execute_block_inputs` takes `BlockExecInput<'b>` which holds `&'b OLTxSegment` and `Option<&'b OLL1ManifestContainer>`. These are references to **owned** types, not Ref/view types. The STF internally accesses fields like `tx_segment.txs()` which returns `&[OLTransaction]` — a slice of owned transactions.

To use Ref types *through* the STF would require either:
- Making STF functions generic over owned/Ref (large effort, touches many crates)
- Or calling `to_owned()` on the parts that enter the STF (practical compromise)

**For the near term**: The realistic approach is to use Ref types for structural verification (hashing, field comparisons) in `execute_block_batch`, and `to_owned()` only the parts that feed into `execute_block_inputs`. This still saves cycles on the hashing paths.

### Not a blocker (but relevant): zkVM IO returns `Vec<u8>` (STR-2172)

`zkaleido::ZkVmEnv::read_buf()` returns `Vec<u8>`. The Ref types would borrow from this `Vec`, which still saves cycles (avoids the full deserialization pass and nested allocations). True zero-copy from guest memory (returning `&[u8]` directly) would require a zkaleido change but is not strictly needed — the `Vec<u8>` workaround is viable.

### Independent: STR-2173 (Tree-hashing inside zkVMs)

ssz-gen already generates `TreeHash` for Ref types. This ticket is likely about ensuring the underlying hash function (SHA256) uses zkVM precompiles efficiently. The dominant cost is `compute_state_root()` (SHA256 of entire OL state, called 2-3x per epoch) which isn't helped by view types but could benefit from precompile optimization.

### Independent: STR-2174 (StableContainers)

`StableContainer` codegen support already exists in ssz-gen. This ticket is about *adopting* them in alpen's protocol types for forward compatibility. Independent of the zero-copy work.

---

## What True Zero-Copy Would Look Like in statements.rs

Today's `process_ol_stf`:
```rust
// Reads bytes from zkVM, then FULLY deserializes into owned structs
let blocks_ssz_bytes = zkvm.read_buf();                          // Vec<u8>
let blocks: Vec<OLBlock> = Vec::<OLBlock>::from_ssz_bytes(&blocks_ssz_bytes).unwrap();  // full deser

// Later, in execute_block_batch, hashing RE-SERIALIZES the owned struct:
let blkid = block.header().compute_blkid();  // calls as_ssz_bytes() internally -> hash
```

With zero-copy Ref types:
```rust
// Read raw bytes, create a view — NO deserialization
let blocks_ssz_bytes = zkvm.read_buf();                          // Vec<u8>
let blocks_ref = VariableListRef::<OLBlockRef, MAX>::from_ssz_bytes(&blocks_ssz_bytes).unwrap();  // validates structure only

// Hashing uses the ORIGINAL bytes — no re-serialization
let header_ref: OLBlockHeaderRef = block_ref.header()?;
let blkid = header_ref.compute_blkid();  // hash::raw(self.as_bytes()) — direct

// Only to_owned() when entering the STF
let tx_seg: OLTxSegment = block_ref.body()?.tx_segment()?.to_owned();
```

---

## Summary: Dependencies and Execution Order

```
ssz-gen: add as_bytes() to container Ref codegen
    |
    v
alpen: add compute methods to OLBlockHeaderRef / OLBlockBodyRef
    |
    v
alpen: rewrite process_ol_stf + execute_block_batch to use Ref types
         (to_owned() only before entering STF)
    |
    v (future, large effort)
alpen: make STF generic over owned/Ref types


Parallel / independent:
  +-- zkaleido (STR-2172): IO layer returning &[u8] instead of Vec<u8>
  +-- ssz-gen (STR-2173): tree-hash optimization for zkVM precompiles
  +-- alpen (STR-2174): adopt StableContainers in protocol types
```

| Step | What                                             | Where                                  | Effort  | Status                                    |
| ---- | ------------------------------------------------ | -------------------------------------- | ------- | ----------------------------------------- |
| 0    | Base checkpoint guest code (owned types)         | `checkpoint-new/src/statements.rs`     | Done    | STR-2129 (functional, not optimized)      |
| 0.5  | Rust-level refactoring (borrow instead of clone) | Same file                              | Done    | STR-2129 (current branch)                 |
| 1    | Add `as_bytes()` to container Ref codegen        | ssz-gen `ssz_codegen/src/types/mod.rs` | ~1 line | **Not started — blocks everything below** |
| 2    | Add compute methods to Ref types                 | `ol/chain-types/src/block.rs`          |         | **Not started**, blocked by step 1        |
| 3    | Use Ref types in checkpoint guest                | `checkpoint-new/src/statements.rs`     |         | **Not started**, blocked by step 2        |
| 4    | Make STF generic over owned/Ref                  | `ol/stf/`, many crates                 |         | **Not started**, blocked by step 3        |

**I am putting STR-2129 on hold** after step 0.5 — the remaining zero-copy work (steps 1-3) is blocked by the ssz-gen `as_bytes()` gap. Once that is resolved, I can proceed with steps 2 and 3 to complete the zero-copy integration in the checkpoint guest.



### Ticket Breakdown
The three Jira tickets (STR-2172, STR-2173, STR-2174) represent infrastructure work needed in `ssz-gen` and zkaleido.


| Ticket       | Title                                                  | What It Covers                                                                  | Blocks Zero-Copy?                                          |
| ------------ | ------------------------------------------------------ | ------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| **STR-2172** | SSZ zero-copy views IO in zkVMs                        | Enable Ref types in zkVM IO, pass raw SSZ bytes through without deserialization | Yes, eliminates an extra memcpy of the entire input buffer |
| **STR-2173** | SSZ tree-hashing inside zkVMs                          | Make SSZ tree-hash (Merkle hashing) work efficiently inside zkVM guests         | No, independent optimization                               |
| **STR-2174** | SSZ forward-compatible containers (`StableContainer`s) | Adopt `StableContainer` for protocol-upgradable SSZ types                       | No, independent of zero-copy                               |

All three are in **Draft** status, unassigned, with no description or comments.
