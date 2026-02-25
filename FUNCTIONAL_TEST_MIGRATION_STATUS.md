# Codebase Ground Reality & Test Migration Status

> Generated: 2026-02-23
> Verified against: commit `521bad6f3` (HEAD of `main`)

---

## Table of Contents

- [1. The Architecture Split](#1-the-architecture-split)
- [2. strata Binary — Standalone OL Client](#2-strata-binary--standalone-ol-client)
- [3. alpen-client Binary — EE Client with Embedded Reth](#3-alpen-client-binary--ee-client-with-embedded-reth)
- [4. Snark Accounts — The Bridge Between OL and EE](#4-snark-accounts--the-bridge-between-ol-and-ee)
- [5. Data Flow: End-to-End Transaction Lifecycle](#5-data-flow-end-to-end-transaction-lifecycle)
- [6. What Changed: Old vs New (Crate-by-Crate)](#6-what-changed-old-vs-new-crate-by-crate)
- [7. RPC Surface Area Comparison](#7-rpc-surface-area-comparison)
- [8. Functional Test Frameworks Comparison](#8-functional-test-frameworks-comparison)
- [9. Jira Epic: STR-2085 Ticket Status](#9-jira-epic-str-2085-ticket-status)
- [10. Migration Feasibility per Ticket](#10-migration-feasibility-per-ticket)

---

## 1. The Architecture Split

### What Happened

The codebase diverged from the v0.2.0-rc release branch around May 2025. Since then, 415 commits on `main` rewrote how the system is composed. **None of the release tags (v0.1.x, v0.2.0-rc1–rc9) are ancestors of current `main`.**

### Before: Monolithic `strata-client`

```
strata-client (ONE binary)
├─ ASM Worker (L1 block processing)
├─ CSM Worker (client state tracking)
├─ Bitcoin I/O (reader, writer, broadcaster)
├─ Consensus logic + sync manager
├─ Old sequencer (template manager, checkpoint tracker)
├─ EE Control (talks to external alpen-reth via eectl/evmexec RPC)
├─ L2 sync (el_sync.rs — pushes blocks to Reth via Engine API)
└─ All RPC endpoints (strata_* namespace)

alpen-reth (SEPARATE binary)
└─ Standard Reth node with Alpen EVM precompiles
```

EE control was tightly coupled: `strata-client` called `RpcExecEngineCtl<EngineRpcClient>` to submit payloads to `alpen-reth` over JSON-RPC. The `el_sync.rs` module did binary search to find where Reth diverged, then replayed blocks sequentially.

### After: `strata` + `alpen-client`

```
strata (NEW binary — pure OL)             alpen-client (NEW binary — pure EE)
├─ ASM Worker                              ├─ Reth node (EMBEDDED, not external)
├─ CSM Worker                              ├─ OL Tracker (polls strata via RPC)
├─ Bitcoin I/O                             ├─ Engine Control (Reth fork choice)
├─ OL Chain Worker (new STF)               ├─ Gossip protocol (RLPx)
├─ OL Mempool (new)                        ├─ Sequencer pipeline:
├─ OL Block Assembly (new)                 │  ├─ Batch Builder
├─ OL Checkpoint Service (new)             │  ├─ Batch Lifecycle
├─ Fork Choice Manager                     │  ├─ DA Pipeline (state diff → L1)
└─ OL RPC (ol_* namespace)                 │  └─ Update Submitter (snark updates → OL)
                                           └─ Ethereum RPC (eth_* namespace)
         ←── RPC bridge ──→
    strata exposes ol_* endpoints
    alpen-client polls them via OLClient trait
```

Two independent processes. `strata` handles consensus + L1 anchoring. `alpen-client` handles EVM execution + DA. They communicate over RPC. Reth is no longer a separate process — it's embedded inside `alpen-client`.

### What Stayed the Same

The **ASM (Anchor State Machine)** is identical in both old and new. Same crates (`asm/common`, `asm/stf`, all subprotocols). Same L1 block processing. Same subprotocol IDs (Admin=0, Checkpoint=1, Bridge=2, Execution DA=3, Debug=254). The ASM worker context (`crates/consensus-logic/src/asm_worker_context.rs`) is shared by both `strata-client` and `strata`.

The `consensus-logic` crate is used by both old and new binaries. It provides the ASM worker, CSM worker, fork choice manager, and genesis logic.

### What's Deprecated

Commit `521bad6f3` ("deprecate old code") added `#[deprecated]` annotations to:

| Component | Deprecated | Replacement |
|-----------|-----------|-------------|
| `L2BlockManager` | `crates/storage/src/managers/l2.rs:18` | `OLBlockManager` |
| `CheckpointManager` | `crates/storage/src/managers/checkpoint.rs:15` | `OLCheckpointManager` |
| `L2BlockDatabase` trait | `crates/db/types/src/traits.rs:156` | `OLBlockDatabase` |
| `CheckpointDatabase` trait | `crates/db/types/src/traits.rs:204` | `OLCheckpointDatabase` |
| `CheckpointEntry` | `crates/db/types/src/types.rs:260` | `OLCheckpointEntry` |
| `StrataApi` trait | `crates/rpc/api/src/lib.rs:26` | `OLClientRpc` + `OLFullNodeRpc` |
| `StrataSequencerApi` trait | `crates/rpc/api/src/lib.rs:179` | `OLSequencerRpc` |
| `RpcClientStatus` | `crates/rpc/types/src/types.rs:362` | `RpcOLChainStatus` |

All deprecation messages say "use OL/EE-decoupled replacement."

---

## 2. strata Binary — Standalone OL Client

**Binary**: `bin/strata/` | **Version**: 0.1.0 | **Feature flags**: `default = ["sequencer"]`

### What It Does

Pure Orchestration Layer node. Processes L1 blocks through ASM, maintains OL chain state, produces OL blocks (sequencer mode), builds checkpoints. **No EVM execution. No Reth. No EE state.**

### CLI Arguments

```
strata
  -c, --config PATH              # Required: config TOML
  -d, --datadir PATH             # Optional: data directory override
  --sequencer                    # Flag: run as sequencer
  --rollup-params PATH           # Rollup params JSON
  --ol-params PATH               # OL genesis params JSON
  --rpc-host STRING              # RPC bind host
  --rpc-port u16                 # RPC bind port
  -k, --sequencer-key PATH       # Signing key (sequencer only)
  -i, --duty-poll-interval u64   # Duty poll ms (sequencer only)
  -o, --overrides STRING[]       # TOML overrides (e.g. -o btcio.reader.client_poll_dur_ms=1000)
```

Config override precedence: base TOML → env vars → CLI flags → `-o` overrides.

### Service Startup Sequence

Defined in `bin/strata/src/services.rs:176-232`. Order is strict — each step depends on the previous:

```
1. ASM Worker
   └─ spawn_asm_worker_with_ctx(&nodectx)
   └─ Processes L1 blocks, produces AnchorState + AsmManifest

2. CSM Worker
   └─ spawn_csm_listener_with_ctx(&nodectx, asm_handle.monitor())
   └─ Tracks client state at L1 boundaries

3. Bitcoin Reader Task [CRITICAL]
   └─ bitcoin_data_reader_task(bitcoin_client, storage, config, params, status, asm_handle)
   └─ Polls bitcoind, submits L1 blocks to ASM
   └─ MUST run before genesis — genesis needs the ASM manifest from L1

4. Genesis Init [BLOCKING, up to 60s]
   └─ check_and_init_genesis(storage, ol_params)
   └─ Waits for ASM to produce genesis manifest (polls every 1s, 60 attempts)
   └─ Creates OL genesis block, state, epoch summary
   └─ Stores initial CSM update with L1 commitment

5. Mempool
   └─ MempoolBuilder::new(config, storage, status_channel, current_tip).launch()
   └─ Initialized from chain tip (status channel) or genesis block fallback

6. Chain Worker
   └─ start_chain_worker_service_from_ctx(&nodectx)
   └─ Executes OL blocks through STF (strata_ol_stf::verify_block)
   └─ Publishes epoch summaries via watch channel

7. OL Checkpoint Service
   └─ OLCheckpointBuilder::new()
        .with_epoch_summary_receiver(chain_worker_handle.subscribe_epoch_summaries())
        .launch()
   └─ Subscribes to epoch completions, builds checkpoints

8. (Sequencer only) L1 Broadcaster + Envelope Writer + OL Block Assembly
   └─ spawn_broadcaster_task() — L1 tx confirmation tracking
   └─ start_envelope_task() — L1 envelope/inscription creation
   └─ BlockasmBuilder::new(params, storage, mempool, epoch_sealing, state, config).launch()

9. Fork Choice Manager
   └─ Manages unfinalized OL blocks
   └─ Processes FcmEvent::NewFcmMsg (new block) and FcmEvent::NewStateUpdate (finalization)
```

After all services: RPC server starts, then (sequencer only) the signer worker with duty fetcher + executor.

### Genesis Flow

`bin/strata/src/genesis.rs` + `crates/ol/genesis/src/lib.rs`:

1. Bitcoin reader feeds L1 blocks to ASM worker
2. ASM processes genesis L1 block, produces manifest
3. `wait_for_genesis_manifest()` polls `storage.l1().get_block_manifest(block_id)` every 1s
4. `build_genesis_artifacts_with_manifest()`:
   - Creates `OLState` from `OLParams` (genesis accounts from params)
   - Wraps genesis manifest in `BlockComponents`
   - Runs OL STF on genesis block
   - Stores: OL block, toplevel OL state, epoch 0 summary, account genesis epochs

### OL Block Assembly (Sequencer)

`crates/ol/block-assembly/src/service.rs`:

The `BlockasmService` processes three commands:
- `GenerateBlockTemplate(config)` — pulls txs from mempool, builds block, caches with 5min expiry
- `GetBlockTemplate(id)` — retrieves cached template
- `CompleteBlockTemplate(id, completion_data)` — applies sequencer signature, stores finalized block

The signer worker (`bin/strata/src/sequencer/signer.rs`) polls for duties, signs templates, and submits checkpoint signatures.

### OL RPC Endpoints

Defined in `crates/ol/rpc/api/src/lib.rs`, implemented in `bin/strata/src/rpc/node.rs`:

**OLClientRpc** (all nodes):
- `ol_getAcctEpochSummary(account_id, epoch)` → epoch commitment + account operations
- `ol_getChainStatus()` → `{latest, confirmed, finalized}` block commitments
- `ol_getBlocksSummaries(account_id, start_slot, end_slot)` → per-block account state
- `ol_getSnarkAccountState(account_id, block_or_tag)` → snark account proof state
- `ol_getAccountGenesisEpochCommitment(account_id)` → genesis epoch for account
- `ol_submitTransaction(tx)` → validates + submits to mempool → tx_id

**OLFullNodeRpc** (all nodes):
- `ol_getRawBlocksRange(start, end)` → raw OL block data
- `ol_getRawBlockById(block_id)` → single raw block

**OLSequencerRpc** (sequencer only):
- `ol_getSequencerDuties()` → `Vec<Duty>`
- `ol_completeBlockTemplate(template_id, completion)` → finalized block ID
- `ol_completeCheckpointSignature(epoch, signature)` → ack

### Status Channel

All services synchronize through `StatusChannel`:
- ASM → CSM: L1 block progress
- Chain Worker: OL block tips, epoch progress
- Checkpoint: Finalized epochs
- FCM: Unfinalized block tracking

The status channel is the synchronization backbone — mempool reads chain tip from it, FCM reads finalized epochs, sequencer duty fetcher reads chain progress.

---

## 3. alpen-client Binary — EE Client with Embedded Reth

**Binary**: `bin/alpen-client/` | **Version**: 0.3.0-alpha.1 | **Feature flags**: `default = ["sequencer"]`

### What It Does

Custom Reth node for EVM execution. Tracks OL state via RPC, builds batches, posts DA to L1, submits snark account updates back to OL. Reth is **embedded** — not a separate process.

### Two Operational Modes

1. **Follower** (no `--sequencer`): Tracks OL consensus, applies to Reth fork choice. No block production.
2. **Sequencer** (`--sequencer`): All of above + batch building, block assembly, DA pipeline, L1 broadcasting, update submission.

### CLI Arguments

```
alpen-client
  --datadir PATH
  --ol-client-url URL           # Strata OL node RPC (http/ws/wss)
  --dummy-ol-client             # Use mock OL client (testing without strata)
  --sequencer                   # Enable sequencer mode
  --sequencer-pubkey HEX        # 32-byte pubkey for gossip validation
  --ee-da-magic-bytes HEX       # 4-byte SPS-50 magic prefix (sequencer+DA)
  --btc-rpc-url URL             # Bitcoin RPC (sequencer+DA)
  --btc-rpc-user STRING
  --btc-rpc-password STRING
  --l1-reorg-safe-depth INT     # L1 confirmations (default: 6)
  --genesis-l1-height INT       # First L1 block (default: 0)
  --batch-sealing-block-count INT # Blocks per batch (default: 100)
  + all standard Reth CLI args (--http, --port, --p2p-secret-key, etc.)
```

`SEQUENCER_PRIVATE_KEY` env var required for sequencer mode.

### OL Client Abstraction

`bin/alpen-client/src/ol_client.rs` + `bin/alpen-client/src/rpc_client.rs`:

Two implementations selected at runtime:

**RpcOLClient** (production):
- Supports HTTP/HTTPS and WS/WSS URLs (auto-detects, defaults to WS if no scheme)
- Retry with exponential backoff on all RPC calls
- Implements both `OLClient` and `SequencerOLClient`

**DummyOLClient** (testing with `--dummy-ol-client`):
- Returns minimal valid responses (empty inbox, zero proof state)
- Deterministic block commitment generation from slot numbers
- Used by all current `alpen-client` functional tests (no real OL connection)

### Spawned Tasks

`bin/alpen-client/src/main.rs:353-531`:

**All modes:**
- `ol_tracker_task` — polls OL for epoch updates every 1s
- `engine_control_task` — applies consensus state to Reth fork choice
- `gossip_task` — P2P block propagation via custom RLPx subprotocol

**Sequencer only:**
- `exec_chain_task` — tracks canonical EE chain, handles orphans/reorgs
- `ol_chain_tracker_task` — fetches OL inbox messages for block building
- `block_builder_task` — assembles L2 blocks with deposits from OL inbox
- `batch_builder_task` — accumulates blocks into batches (FixedBlockCountSealing)
- `batch_lifecycle_task` — manages batch state machine (Sealed → DaPending → DaComplete → ProofPending → ProofReady)
- `update_submitter_task` — builds SnarkAccountUpdate from proved batches, submits to OL
- `l1_broadcaster_task` — broadcasts transactions to Bitcoin
- `chunked_envelope_watcher_task` — monitors chunked DA envelope confirmations

### OL Tracker

`crates/alpen-ee/ol-tracker/src/task.rs`:

Polling loop every `poll_wait_ms` (default 1000ms):

1. Call `ol_client.chain_status()` to get OL's latest epoch
2. Compare with local best epoch:
   - `local > ol`: Warn, noop (unusual)
   - `local == ol`, same terminal block: Noop (in sync)
   - `local == ol`, different block: **Reorg detected**
   - `local < ol`: **Extend** — fetch missing epochs
3. For each new epoch: call `ol_client.epoch_summary(epoch_num)`, verify chain continuity
4. Apply epoch operations to local EE account state
5. Persist state atomically
6. Notify watchers: `ol_status_watcher` (for update submitter) and `consensus_watcher` (for engine control)

### Engine Control

`crates/alpen-ee/engine/src/control.rs`:

Dual input sources driving Reth's `ForkchoiceState`:

1. **Preconf (Gossip/Sequencer)** → `preconf_rx` watch channel → updates head block hash immediately
2. **Consensus (OL State)** → `consensus_rx` from OL tracker → updates safe + finalized hashes

Decision logic (`forkchoice_state_from_consensus`, line 28):
- If safe block is in Reth's canonical chain → keep preconf head (faster finality)
- If safe block is NOT in canonical chain → use OL's safe block as head (OL is authoritative)

### Gossip Protocol

`bin/alpen-client/src/gossip.rs`:

Custom RLPx subprotocol for block propagation:
- Sequencer signs messages with private key
- Peers verify against expected `sequencer_pubkey`
- Block number used as sequence number (prevents duplicates/replays)
- On receiving valid gossip: forward block hash + number to `preconf_tx`, re-broadcast to other peers
- On producing canonical block (sequencer): sign and broadcast to all peers

### DA Pipeline (End-to-End)

`crates/alpen-ee/da/`:

Two layered providers:

**StateDiffBlobProvider** (`blob_provider.rs`):
1. Look up batch → get block range
2. For each block: get Reth state diff via `StateDiffGenerator` exex
3. Aggregate diffs via `BatchBuilder`
4. Filter out already-published bytecodes (deduplication)
5. Read last block's header for chain reconstruction
6. Return `DaBlob` with batch_id, evm_header, state_diff

**ChunkedEnvelopeDaProvider** (`envelope_provider.rs`):
1. Get blob from StateDiffBlobProvider
2. Split into chunks via `prepare_da_chunks()`
3. Create ChunkedEnvelopeEntry (unsigned, goes to envelope handle)
4. Envelope handle signs + submits individual chunks to L1 via broadcaster
5. Monitor: Unsigned → NeedsResign → Unpublished → CommitPublished → Published → Confirmed → **Finalized**
6. On finalized: build `L1DaBlockRef` list (which L1 blocks contain the DA)

### Known Gaps

| Location | Issue |
|----------|-------|
| `main.rs:113` | Config/params read from file not implemented (hardcoded) |
| `main.rs:118` | AccountId hardcoded to `[1u8; 32]` |
| `main.rs:202` | Using `NoopProver` — no real proof generation |
| `main.rs:530-531` | Proof generation + OL posting marked TODO |
| `common/types/prover.rs:6` | Proof type is `Vec<u8>` placeholder |
| `block_builder/config.rs:10` | Bridge gateway account hardcoded to `AccountId::special(1)` |
| `ol_chain_tracker/init.rs:20,40` | Missing retry logic and slot range chunking |
| `exec-chain/task.rs:213,217` | Deep reorg beyond finalization not handled |

---

## 4. Snark Accounts — The Bridge Between OL and EE

This is the core mechanism that connects the two binaries. It's not just a data structure — it's the entire trust model for how EE state gets committed to OL.

### What Is a Snark Account

A **snark account** is an actor-like entity on the OL that can receive inbox messages, maintain proven state, and send outputs (transfers + messages) to other accounts. It's identified by an `AccountId` (32 bytes).

`alpen-client` operates AS a snark account on the OL. It:
1. Receives inbox messages from OL (e.g., deposits, epoch operations)
2. Processes them through EVM execution
3. Builds a proof that the state transition was valid
4. Submits a `SnarkAccountUpdate` back to OL with the new state commitment

### Snark Account State

`crates/snark-acct-types/src/state.rs`:

```
SnarkAccountState
├─ update_vk: Vec<u8>          // Verification key for update proofs
├─ proof_state: ProofState
│  ├─ inner_state: Hash        // Tree-hash of account state
│  └─ next_inbox_msg_idx: u64  // Next inbox message to process
├─ seq_no: Seqno               // Monotonic update counter (u64)
└─ inbox_mmr: MmrState         // Merkle Mountain Range of received messages
```

**Seqno** is critical: it's a monotonically increasing counter. Each update must use the next expected seq_no. OL rejects out-of-order or duplicate updates.

### The Update Flow (Batch → OL)

This is the full path from EVM block execution to OL state commitment:

**Step 1: Batch Sealing** (`crates/alpen-ee/sequencer/src/batch_builder/`)
- Blocks accumulate in the batch builder
- `FixedBlockCountSealing` policy seals after N blocks
- Sealed batch enters lifecycle pipeline

**Step 2: DA Posting** (`crates/alpen-ee/sequencer/src/batch_lifecycle/`)
- Batch lifecycle state machine: `Sealed → DaPending → DaComplete → ProofPending → ProofReady`
- DA is posted to L1 via `ChunkedEnvelopeDaProvider`
- Waits for L1 finalization (configurable `l1_reorg_safe_depth`)

**Step 3: Proof Generation** (currently NoopProver)
- `ProofPending` → prover generates validity proof → `ProofReady`
- Currently using `NoopProver` (accepts all batches without real proof)
- Proof would normally be SP1 STARK proving EVM execution correctness

**Step 4: Update Building** (`crates/alpen-ee/sequencer/src/update_submitter/update_builder.rs`)

```
build_update_from_batch(batch, proof_id, exec_storage, prover):
  1. Fetch all blocks in batch from exec_storage
  2. Get proof bytes from prover
  3. seq_no = batch_idx - 1 (genesis batch 0 never submitted)
  4. From last block: extract inner_state (tree hash of account state)
  5. From last block: extract next_inbox_msg_idx
  6. Across all blocks: accumulate processed messages + outputs
  7. Build UpdateOperationData:
     ├─ seq_no
     ├─ proof_state: ProofState(inner_state, next_inbox_msg_idx)
     ├─ processed_messages: Vec<MessageEntry>
     ├─ ledger_refs: LedgerRefs (empty for now)
     ├─ outputs: UpdateOutputs
     │  ├─ transfers: Vec<OutputTransfer>  (max 64)
     │  └─ messages: Vec<OutputMessage>    (max 64)
     └─ extra_data: UpdateExtraData(block_tip, processed_inputs, padding)
  8. Return SnarkAccountUpdate(operation, proof_bytes)
```

**Step 5: Submission** (`crates/alpen-ee/sequencer/src/update_submitter/task.rs`)

The update submitter task has three triggers:
- `batch_ready_rx.changed()` — new batch reached ProofReady
- `ol_status_rx.changed()` — OL chain status update (new accepted seq_no)
- 60-second polling fallback

Processing loop (`process_ready_batches`):
1. Query OL for current account state: `ol_client.get_latest_account_state()` → gets current `seq_no`
2. Calculate `next_batch_idx = seq_no + 1`
3. Evict cache entries for already-accepted batches
4. Iterate batches from `next_batch_idx` forward:
   - Must be `ProofReady` status
   - Must be submitted in order (stops at first non-ready)
5. For each: build or retrieve cached `SnarkAccountUpdate`
6. Call `ol_client.submit_update(update)` — serializes to SSZ, wraps in `RpcOLTransaction`, submits to OL mempool

**Step 6: OL Processing**

On the `strata` side, the submitted `RpcOLTransaction` with `SnarkAccountUpdateTxPayload`:
1. Enters OL mempool (`ol_submitTransaction`)
2. Gets included in next OL block by block assembly
3. OL STF verifies: seq_no matches, proof validates against update_vk
4. Applies outputs (transfers, messages) to OL ledger
5. Updates snark account state (new proof_state, incremented seq_no)
6. Epoch summary includes this update

**Step 7: OL Tracker Sees the Update**

Back in `alpen-client`, the OL tracker polls for new epochs:
1. `ol_client.epoch_summary(epoch)` returns updates that happened in this epoch
2. `apply_epoch_operations()` applies them to local EE account state
3. Updates consensus heads (safe, finalized)
4. Engine control applies new fork choice to Reth

### Key Invariants

- Updates submitted strictly in `batch_idx` order. OL deduplicates replays via seq_no.
- `seq_no = batch_idx - 1` (genesis batch 0 is never submitted)
- DA must be finalized on L1 before proof is generated
- Proof must be ready before update can be submitted
- OL tracker validates chain continuity on every epoch — detects reorgs

---

## 5. Data Flow: End-to-End Transaction Lifecycle

### User Transaction (EVM side)

```
User sends ETH transfer to alpen-client via eth_sendRawTransaction
    ↓
Reth mempool accepts transaction
    ↓
Sequencer's payload builder creates new block (with deposits from OL inbox)
    ↓
Reth executes block (EVM state transitions)
    ↓
StateDiffGenerator exex produces per-block state diff
    ↓
Gossip task broadcasts new block to fullnodes via RLPx
    ↓
Batch builder accumulates block into current batch
    ↓
(After N blocks) Batch sealed → enters lifecycle pipeline
```

### Batch → L1 DA → OL

```
Batch lifecycle: Sealed
    ↓
StateDiffBlobProvider aggregates state diffs across batch
    ↓
ChunkedEnvelopeDaProvider splits blob into L1-sized chunks
    ↓
L1 broadcaster inscribes chunks into Bitcoin witness data (SPS-50 tagged)
    ↓
Batch lifecycle: DaPending → (L1 finalization) → DaComplete
    ↓
Prover generates validity proof (currently NoopProver)
    ↓
Batch lifecycle: ProofPending → ProofReady
    ↓
Update submitter builds SnarkAccountUpdate
    ↓
Submits to strata OL via ol_submitTransaction RPC
    ↓
OL mempool → OL block assembly → OL block execution
    ↓
OL state updated: snark account seq_no incremented, proof_state committed
```

### OL → EE Consensus

```
OL tracker polls strata: ol_getChainStatus()
    ↓
Detects new epoch → fetches ol_getAcctEpochSummary()
    ↓
Applies epoch operations to local EE state
    ↓
Notifies engine control via consensus_rx watch channel
    ↓
Engine control updates Reth's ForkchoiceState:
  - head: from gossip preconf (latest block) or OL safe block
  - safe: from OL confirmed epoch
  - finalized: from OL finalized epoch
    ↓
Reth applies fork choice: marks blocks as safe/finalized
```

---

## 6. What Changed: Old vs New (Crate-by-Crate)

### Crates Used ONLY by Old `strata-client`

| Crate | What It Did | Replaced By |
|-------|-------------|-------------|
| `strata_eectl` | `ExecEngineCtl` trait — abstraction over Reth Engine API | Reth embedded directly in alpen-client |
| `strata_evmexec` | `RpcExecEngineCtl<EngineRpcClient>` — RPC-based EL control | `AlpenRethExecEngine` in alpen-ee-engine |
| `strata_sequencer` (old) | Template manager, checkpoint tracker, duty extraction | Split into `ol_block_assembly` + `ol_checkpoint` + `ol_sequencer` |
| `strata_sync` | Full node L2 sync via RPC from sequencer | Removed entirely — EE syncs via Reth, OL doesn't do full-node sync |
| `strata_rpc_api` (deprecated) | `StrataApi`, `StrataSequencerApi` traits | `OLClientRpc`, `OLSequencerRpc` in `ol/rpc/api` |
| `strata_rpc_types` (deprecated) | `RpcClientStatus`, `RpcBlockHeader`, etc. | `RpcOLChainStatus`, `RpcAccountEpochSummary` in `ol/rpc/types` |
| `strata_ol_chain_types` (old, flat) | `L2Block`, `L2BlockBundle`, `L2BlockId` | `OLBlock`, `OLTransaction` in `ol/chain-types` (SSZ-generated) |

### Crates Used ONLY by New Binaries

| Crate | Binary | Purpose |
|-------|--------|---------|
| `strata_ol_block_assembly` | strata | OL block construction from mempool |
| `strata_ol_mempool` | strata | OL transaction pool (new — old had none) |
| `strata_ol_checkpoint` | strata | OL checkpoint building |
| `strata_ol_stf` | strata | OL state transition function |
| `strata_ol_state_types` | strata | Account-centric OL state (ledger, epochs, snark accounts) |
| `strata_ol_sequencer` | strata | Simplified duty extraction |
| `strata_ol_genesis` | strata | OL genesis with OLParams |
| `strata_ol_params` | strata | OL-specific parameters |
| `strata_ol_rpc_api` | strata | New RPC trait definitions |
| `strata_ol_rpc_types` | strata | New RPC response types |
| `strata_chain_worker_new` | strata | New OL block processor |
| `strata_node_context` | strata | Centralized node initialization |
| `alpen_ee_*` (11 crates) | alpen-client | Entire EE layer |

### Crates Shared by Both

| Crate | Usage |
|-------|-------|
| `strata_consensus_logic` | ASM worker, CSM worker, FCM, genesis (identical usage) |
| `strata_btcio` | Bitcoin reader, writer, broadcaster |
| `strata_params` | Rollup parameters |
| `strata_storage` | Storage manager interfaces |
| `strata_db_store_sled` | SledDB implementation |
| `strata_db_types` | Database trait definitions (includes both deprecated + new) |
| `strata_primitives` | Core primitive types |
| `strata_status` | Status channel types |

### Chain Types: Old vs New

**Old** (`crates/ol-chain-types/`): `L2Block`, `L2BlockBundle`, `L2BlockHeader`, `L2BlockId`

**New** (`crates/ol/chain-types/`): All SSZ-generated from `.ssz` schemas:
- `OLBlock`, `OLBlockBody`, `OLBlockHeader`, `OLBlockRef`
- `OLTransaction`, `OLTransactionRef`
- `SnarkAccountUpdateTxPayload`, `GamTxPayload`
- `OLLog`, `OLLogRef`
- `OLL1ManifestContainer`, `OLL1Update`, `OLTxSegment`

Key differences: new types are account-aware (snark account payloads as transaction types), SSZ-serialized (deterministic), and include transaction types for snark updates and GAM operations.

---

## 7. RPC Surface Area Comparison

### Old strata-client RPC (deprecated)

```
strata_protocolVersion()
strata_getBlocksAtIdx(idx)
strata_getClientStatus()
strata_getChainstateRaw(block_id)
strata_getEpochSummary(...)
strata_syncStatus()
strata_getRecentBlocksRange(start, count)
strata_getRawBlockById(block_id)
strata_getCurrentDeposits()                 # Bridge-specific
strata_getCheckpointInfo(idx)               # Checkpoint-specific
strata_getLatestCheckpointIndex()           # Checkpoint-specific

# Sequencer-only:
stratasequencer_getBlockTemplate()
stratasequencer_completeBlockTemplate(...)
stratasequencer_getCheckpointSignature(...)
stratasequencer_completeCheckpointSignature(...)
```

### New strata RPC (ol_* namespace)

```
strata_protocolVersion()

# OLClientRpc:
ol_getAcctEpochSummary(account_id, epoch)
ol_getChainStatus()
ol_getBlocksSummaries(account_id, start_slot, end_slot)
ol_getSnarkAccountState(account_id, block_or_tag)
ol_getAccountGenesisEpochCommitment(account_id)
ol_submitTransaction(tx)

# OLFullNodeRpc:
ol_getRawBlocksRange(start, end)
ol_getRawBlockById(block_id)

# OLSequencerRpc:
ol_getSequencerDuties()
ol_completeBlockTemplate(template_id, completion)
ol_completeCheckpointSignature(epoch, signature)
```

### New alpen-client RPC

Standard Reth `eth_*` endpoints plus custom Alpen extensions. No `strata_*` or `ol_*` endpoints — it's a pure EVM client.

### Impact on Functional Tests

Any old test calling `strata_*` RPC methods needs rewriting:
- `strata_getClientStatus()` → `ol_getChainStatus()` (different response schema)
- `strata_getCurrentDeposits()` → No direct equivalent (bridge is in ASM subprotocol now)
- `strata_getCheckpointInfo()` → `ol_getAcctEpochSummary()` (account-centric, not index-based)
- `strata_syncStatus()` → `ol_getChainStatus()` (different fields)
- `eth_blockNumber()` etc. → Now on alpen-client, not strata

Tests that call `strataee_*` RPCs (4 tests marked "can't port" in STR-2087) have no equivalent — those endpoints don't exist in the new architecture.

---

## 8. Functional Test Frameworks Comparison

### New Framework (`functional-tests-new/`)

**Binaries built**: `cargo build --bin strata --bin alpen-client`

**Class hierarchy**:
```
flexitest.Test
└─ BaseTest (common/base_test.py)
   ├─ StrataNodeTest  → typed get_service() for Bitcoin, Strata
   └─ AlpenClientTest → typed get_service() for AlpenClient
```

**Services** (proper class hierarchy with health checks):
```
flexitest.service.ProcService
└─ RpcService (common/services/base.py)
   ├─ BitcoinService  → create_rpc() → BitcoindClient
   ├─ StrataService   → create_rpc() → JsonRpcClient, wait_for_block_height(), etc.
   └─ AlpenClientService → create_rpc() → JsonRpcClient, get_enode(), wait_for_peers(), etc.
```

**Environments** (5 predefined):
- `basic` → Bitcoin + Strata sequencer (pre-generates 110 blocks)
- `alpen_client` → Bitcoin + AlpenClient sequencer + 1 fullnode (DA enabled)
- `alpen_client_discovery` → pure discv5 discovery mode
- `alpen_client_multi` → 3 fullnodes
- `alpen_client_mesh` → 5 fullnodes, mesh topology

**Port ranges**: AlpenClient 30303-30503, Bitcoin 18443-18543, Strata 19443-19543

**Config flow**: `StrataConfig.as_toml_string()` → config.toml, `RollupParams.as_json_string()` → rollup-params.json, `OLParams.as_json_string()` → ol-params.json

**EVM helpers** (limited): `sign_transfer()`, `deploy_storage_filler()`, `deploy_large_runtime_contract()`

**DA helpers** (rich): `scan_for_da_envelopes()`, `reassemble_blobs_from_envelopes()`, `DaBlob.is_empty_batch()`

### Old Framework (`functional-tests/`)

**Binaries built**: `cargo build -F debug-utils -F test-mode` (builds everything including `strata-client`)

**Class hierarchy**:
```
flexitest.Test
└─ StrataTestBase (envs/testenv.py)
   └─ BaseMixin → sets up self.btc, self.seq, self.reth, self.seqrpc, etc.
      ├─ BridgeMixin → deposit(), withdraw(), fulfill_withdrawal_intents()
      ├─ DbtoolMixin → database inspection commands
      └─ SeqCrashMixin → kill/restart sequencer at specific points
```

**Services**: Generic `flexitest.service.ProcService` with **monkey-patched** `create_rpc()` and `snapshot_datadir()`/`restore_snapshot()` methods. No health check abstraction.

**Environments** (8+): `basic`, `hub1` (seq+fullnode), `prover`, `crash`, and more. Complex setup with `RollupSettings`, `ProverClientSettings`.

**Port ranges**: Different per factory, manually managed.

**Config flow**: `Config.as_toml_string()` → config.toml, `RollupConfig.to_json_string()` → rollup_params.json. No OLParams (didn't exist in old arch).

**EVM helpers** (rich): `EthTransactions` class, `FundedAccount`, `GenesisAccount`, `Web3` integration, contract deployment, bridge precompile interaction.

**Typed waiters** (rich): `StrataWaiter.wait_until_genesis()`, `RethWaiter.wait_until_eth_block_exceeds()`, `ProverWaiter` for proof completion.

### Key Gaps in New Framework

| Capability | Old | New |
|-----------|-----|-----|
| Typed RPC waiters (StrataWaiter, RethWaiter) | Rich, per-service | Generic `wait_until()` only |
| Full EVM tx builder | `EthTransactions` class with Web3 | `sign_transfer()` only, no contract calls |
| Bridge utilities | `BridgeMixin.deposit()`, `withdraw()` | None |
| Crash injection | `SeqCrashMixin` | None |
| Dbtool wrapper | `DbtoolMixin` | None |
| Prover factory/service | `ProverClientSettings` + factory | None |
| Snapshot/restore | Monkey-patched on services | None |
| Multi-node strata env | `hub1` (seq + fullnode) | Strata factory supports both, but no env config for hub |
| Bridge address generation | `gen_ext_btc_address()`, taproot | None |
| Reth service | Separate `RethFactory` | Reth embedded in AlpenClient (correct for new arch) |

---

## 9. Jira Epic: STR-2085 Ticket Status

**Epic**: [STR-2085](https://alpenlabs.atlassian.net/browse/STR-2085) — Migrate functional tests to new binaries
**Assignee**: Ashish | **Status**: Draft | **Created**: 2026-01-15

**End goal**: Migrate → Delete `functional-tests/` → Rename `functional-tests-new/` to `functional-tests`

**Blocks**: [STR-2170](https://alpenlabs.atlassian.net/browse/STR-2170) — Remove unused crates (can't delete old code until tests migrated)
**Related**: [STR-2045](https://alpenlabs.atlassian.net/browse/STR-2045) — Fix functional tests flakiness (Draft)

| Ticket | Summary | Status | Test Count | Key Detail |
|--------|---------|--------|------------|------------|
| [STR-2086](https://alpenlabs.atlassian.net/browse/STR-2086) | Migrate factories to new binaries | **In Progress** | Infra | Factories already exist for `strata` + `alpen-client` |
| [STR-2087](https://alpenlabs.atlassian.net/browse/STR-2087) | Migrate EE tests | **In Review** | 5 port + 4 new | 4 tests explicitly "can't port" (need `strataee_*` RPCs) |
| [STR-2088](https://alpenlabs.atlassian.net/browse/STR-2088) | Migrate EE Precompiles | Draft | 6 | Half are prover-dependent (BLS, point eval, blockhash) |
| [STR-2089](https://alpenlabs.atlassian.net/browse/STR-2089) | Migrate Bridge | Draft | 6 | Full deposit/withdrawal + indirect withdrawals + bridge RPC |
| [STR-2090](https://alpenlabs.atlassian.net/browse/STR-2090) | Migrate BtcIO | Draft | 6 | Read, broadcast, connect, inscribe, reorg, checkpoint resubmit |
| [STR-2091](https://alpenlabs.atlassian.net/browse/STR-2091) | Migrate Sync & Consensus | Draft | 6 | Genesis, fullnode lag, restart, L1 finalization, reorg, RPC sync |
| [STR-2092](https://alpenlabs.atlassian.net/browse/STR-2092) | Migrate Prover + Crash | Draft | **20** | Bundles 16 prover + 4 crash tests. Biggest ticket. |
| [STR-2093](https://alpenlabs.atlassian.net/browse/STR-2093) | Migrate Database (dbtool) | Draft | 9 | Chainstate revert, checkpoint revert, syncinfo validation |
| [STR-2094](https://alpenlabs.atlassian.net/browse/STR-2094) | Migrate OL tests | Draft | 4 | **Blocked on snark account creation decision** (Slack thread) |
| [STR-2095](https://alpenlabs.atlassian.net/browse/STR-2095) | Migrate RPC tests | Draft | 3 | Notes "two clients now" — need to split per binary |
| [STR-2096](https://alpenlabs.atlassian.net/browse/STR-2096) | Migrate Load & Perf | Draft | 1 | basic_load (was disabled) |
| [STR-2097](https://alpenlabs.atlassian.net/browse/STR-2097) | Migrate TX Management | Draft | 1 | `forward_tx` → already covered by `test_transaction_mempool_propagation` |
| [STR-2098](https://alpenlabs.atlassian.net/browse/STR-2098) | Migrate Misc | Draft | 1 | keepalive stub → already in new suite |
| [STR-2099](https://alpenlabs.atlassian.net/browse/STR-2099) | Delete old TN1 binaries | Draft | Cleanup | Blocked on all migration completing |
| [STR-2101](https://alpenlabs.atlassian.net/browse/STR-2101) | Migrate Client Mgmt | Draft | 2 | Restart partially covered. Status needs porting. |
| [STR-2114](https://alpenlabs.atlassian.net/browse/STR-2114) | Python code improvements | Draft | Infra | `ty` type checker, RPC type defs, logging |

---

## 10. Migration Feasibility per Ticket

### Already Done or Trivially Closeable

| Ticket | Why |
|--------|-----|
| STR-2086 (Factories) | `StrataFactory` and `AlpenClientFactory` already target new binaries |
| STR-2097 (TX Mgmt) | `test_transaction_mempool_propagation` in new suite covers `forward_tx` |
| STR-2098 (Misc) | keepalive stub already exists in new suite |

### Ready to Work On (No Architectural Blockers)

| Ticket | Tests | What's Needed |
|--------|-------|---------------|
| STR-2087 (EE) | 5 port + 4 new | **In Review** — nearly done |
| STR-2088 (Precompiles) | 3 non-prover | Schnorr + bridge precompile need contract deployment helpers in new framework |
| STR-2090 (BtcIO) | 6 | Bitcoin I/O is in `strata`. Needs: BtcIO test patterns adapted for `strata` binary (not `strata-client`). Bitcoin factory already exists. |
| STR-2093 (Database) | 9 | `strata-dbtool` is standalone. Needs: subprocess wrapper for dbtool commands. Service snapshot/restore for before/after comparison. |
| STR-2095 (RPC) | 3 | Straightforward `ol_*` RPC calls. Old `rpc_exec_update` → split between `ol_getChainStatus()` and `eth_*` calls. `rpc_el_inactive` → test alpen-client behavior when strata is down. |
| STR-2101 (Client Mgmt) | 1 remaining | `client_status` needs porting to `ol_getChainStatus()` |
| STR-2114 (Python QoL) | Infra | Independent. Should happen early to benefit all subsequent work. |

### Need Hub Environment First

| Ticket | Tests | What's Needed |
|--------|-------|---------------|
| STR-2091 (Sync) | 6 | Needs `StrataEnvConfig` variant with sequencer + fullnode strata nodes + alpen-client. `StrataFactory.create_node(is_sequencer=False)` already supports fullnode mode. Missing: env config that wires fullnode to sync from sequencer + alpen-client for EE sync tests. |

### Need Architectural Decisions

| Ticket | Blocker |
|--------|---------|
| STR-2094 (OL tests) | Waiting on snark account creation approach. CL block witness, checkpoint rejection, reorg block production — all need to understand how snark accounts are created in test environments. Currently `alpen-client` hardcodes `AccountId::new([1u8; 32])`. The Slack discussion (linked in ticket) is about making this configurable. |
| STR-2089 (Bridge) | Bridge deposit/withdrawal in new arch goes through ASM bridge subprotocol → OL snark account inbox → EE execution. Old tests used `BridgeMixin` which called `strata_getCurrentDeposits()` (deprecated). New flow: deposit appears as inbox message on snark account. Need bridge utilities that understand new flow. |

### Hard Blocked — Don't Start Yet

| Ticket | Why |
|--------|-----|
| STR-2092 (Prover + Crash) | **20 tests. Should be split.** Prover: needs SP1 builder, prover-client factory, `PROVER_TEST=1` gating. Currently `alpen-client` uses `NoopProver`. Crash: needs service snapshot/restore + kill injection for 2-process model. These are completely different infrastructure requirements bundled into one ticket. |
| STR-2099 (Delete old code) | Blocked on all of the above. |

### Suggested First Moves

1. **Close STR-2097 and STR-2098** — work is done.
2. **Land STR-2087** — already in review.
3. **Do STR-2114 (Python improvements)** — type checker + RPC types make everything cleaner.
4. **Build hub environment** for STR-2091 — this validates the 2-binary model works under test.
5. **Ask for STR-2092 to be split** — prover (16 tests) and crash (4 tests) are independent workstreams.
6. **Resolve the snark account creation question** — unblocks STR-2094 and partially STR-2089.
