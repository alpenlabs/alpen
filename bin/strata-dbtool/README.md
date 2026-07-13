# Strata DB Tool

The Strata DB Tool is an offline database inspection and maintenance utility for Alpen nodes. It allows you to inspect, repair, and roll back an Alpen node's database while the node is offline.

## Installation

### From Source

1. Clone the repository:
```bash
git clone https://github.com/alpenlabs/alpen.git
cd alpen
```

2. Build the tool:
```bash
cargo build --release --bin strata-dbtool
```

3. The binary will be available at `target/release/strata-dbtool`

## Usage

The Strata DB Tool operates on an offline Alpen node database. Make sure your Alpen node is stopped before using this tool.

### Basic Syntax

```bash
strata-dbtool [OPTIONS] <COMMAND>
```

### Global Options

- `-d, --datadir <path>` - Data directory of the node whose DB is being inspected (default: `data`). For `ee-*` subcommands, point this at the alpen-client's `--datadir` instead of the strata node's — each invocation is standalone and opens exactly one sled.

## Commands

### `get-syncinfo`
Shows the latest synchronization information including L1/OL tips, epochs, and block status.

```bash
strata-dbtool get-syncinfo [OPTIONS]
```

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)
- `--l1-reorg-safe-depth <depth>` - L1 reorg-safe depth used to derive finalized checkpoint epoch

**Example:**
```bash
strata-dbtool get-syncinfo --l1-reorg-safe-depth 6
```

**Notes:**
- `top_level_state.prev_epoch.status` is derived from OL checkpoint DB at read time.
- `top_level_state.finalized_epoch` is derived from OL checkpoint DB using `--l1-reorg-safe-depth`.

### `get-client-state-update`
Shows client state update information for a given L1 block.

```bash
strata-dbtool get-client-state-update <block_id> [OPTIONS]
```
**Arguments:**
- `block_id` - L1 block ID (hex string)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-client-state-update 42b3fd7680ea6141eec61ae5ae86e41163ab559b6a1ab86c4de9c540a2c5f63f
```

### `get-l1-summary`
Shows a summary of all L1 manifests in the database.

```bash
strata-dbtool get-l1-summary <height_from> [OPTIONS]
```

**Arguments:**
- `height_from` - L1 height to look up the summary about

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

### `get-l1-block`
Shows detailed information about a specific ASM manifest entry stored in the L1 database.

```bash
strata-dbtool get-l1-block <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - L1 block ID (hex string)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-l1-block 42b3fd7680ea6141eec61ae5ae86e41163ab559b6a1ab86c4de9c540a2c5f63f
```

### `get-writer-summary`
Shows a summary of writer database contents including payload and intent entry counts.

```bash
strata-dbtool get-writer-summary [OPTIONS]
```

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-writer-summary
```

### `get-writer-payload`
Shows detailed information about a specific writer payload entry by index.

```bash
strata-dbtool get-writer-payload <index> [OPTIONS]
```

**Arguments:**
- `index` - Payload entry index (number)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-writer-payload 5
```

### `get-broadcaster-summary`
Shows a summary of broadcaster database contents including transaction counts by status.

```bash
strata-dbtool get-broadcaster-summary [OPTIONS]
```

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-broadcaster-summary
```

### `get-broadcaster-tx`
Shows detailed information about a specific broadcaster transaction entry by index.

```bash
strata-dbtool get-broadcaster-tx <index> [OPTIONS]
```

**Arguments:**
- `index` - Transaction entry index (number)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-broadcaster-tx 3
```

### `get-ol-summary`
Shows a summary of OL blocks in the database.

```bash
strata-dbtool get-ol-summary <slot_from> [OPTIONS]
```

**Arguments:**
- `slot_from` - Slot to start scanning from

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-ol-summary 100
```

### `get-ol-blocks-at-slot`
Shows all OL block IDs stored for a slot.

```bash
strata-dbtool get-ol-blocks-at-slot <slot> [OPTIONS]
```

**Arguments:**
- `slot` - OL slot

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-ol-blocks-at-slot 100
```

### `get-ol-block`
Shows detailed information about a specific OL block.

```bash
strata-dbtool get-ol-block <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - OL block ID (hex string)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-ol-block 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
```

### `get-checkpoints-summary`
Shows a summary of all OL checkpoints in the database.

```bash
strata-dbtool get-checkpoints-summary <height_from> [OPTIONS]
```

**Arguments:**
- `height_from` - Start L1 height to query checkpoints from

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-checkpoints-summary 10
```

### `get-checkpoint`
Shows detailed information about a specific OL checkpoint epoch.

```bash
strata-dbtool get-checkpoint <checkpoint_epoch> [OPTIONS]
```

**Arguments:**
- `checkpoint_epoch` - Checkpoint epoch (number)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)
- `--l1-reorg-safe-depth <depth>` - L1 reorg-safe depth used to derive checkpoint status

**Example:**
```bash
strata-dbtool get-checkpoint 5 --l1-reorg-safe-depth 6
```

**Notes:**
- Checkpoint status is reported as canonical `checkpoint.status`:
  `Unsigned`, `Signed`, `Confirmed`, or `Finalized`.
- For `Signed`, output includes `checkpoint.intent_index`.

### `get-epoch-summary`
Shows detailed information about a specific OL epoch summary.

```bash
strata-dbtool get-epoch-summary <epoch> [OPTIONS]
```

**Arguments:**
- `epoch` - epoch

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-epoch-summary 5
```

### `get-ol-state`
Shows the current OL state information.

```bash
strata-dbtool get-ol-state <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - OL block ID (hex string)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)
- `--l1-reorg-safe-depth <depth>` - L1 reorg-safe depth used to derive finalized checkpoint epoch

**Example:**
```bash
strata-dbtool get-ol-state 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b --l1-reorg-safe-depth 6
```

**Notes:**
- `top_level_state.finalized_epoch` is derived from OL checkpoint DB using `--l1-reorg-safe-depth`.

### `revert-ol-state`
Reverts the OL state to a specific block ID.

> [!WARNING]
> 
> **This command can cause irreversible data loss. Always run in dry-run mode first!**
>
> **Full Node:**
> - Can revert up to the last block of the finalized epoch. The command will error if you try to revert to a block earlier than that.

> [!NOTE]
> 
> **Default behavior:**
> - By default, this command runs in **dry-run mode** and shows what would be deleted without making any changes.
> - To actually execute the revert operation, you must explicitly use the `--force` or `-f` flag.

> [!IMPORTANT]
> 
> **Sequencer - Critical Safety Requirements:**
> - **DO NOT revert anything from the previous epoch or earlier.** You can only revert blocks from the current epoch.
> - **DO NOT use the `-c` (--revert-checkpointed-blocks) flag on the sequencer.**
> - The checkpoint for the previous epoch may already be confirmed on L1 or have a proof ready (L1 transactions may already be broadcasted or broadcasted soon). If you delete checkpoints and epoch summaries for the previous epoch and earlier, the sequencer may not be able to restart.

```bash
strata-dbtool revert-ol-state <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - Target OL block ID to revert to (hex string)

**Options:**
- `-f, --force` - Force execution (without this flag, only a dry run is performed)
- `-d, --delete-blocks` - Delete blocks after target block (not just mark as unchecked)
- `-c, --revert-checkpointed-blocks` - Allow reverting blocks inside checkpointed epoch
- `--l1-reorg-safe-depth <depth>` - L1 reorg-safe depth used to derive finalized checkpoint epoch for revert safety checks

**Examples:**

Dry run (default behavior - shows what would be affected):
```bash
strata-dbtool revert-ol-state 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b --l1-reorg-safe-depth 6
```

Actually execute the revert:
```bash
strata-dbtool revert-ol-state --force 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b --l1-reorg-safe-depth 6
```

Execute revert with block deletion:
```bash
strata-dbtool revert-ol-state -f -d 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b --l1-reorg-safe-depth 6
```

## Prover Task Admin

> [!WARNING]
>
> These commands mutate the prover-task store and the checkpoint-proof receipt
> store. Stop the node before using them — concurrent writes from a running
> prover will conflict with these edits and may corrupt state.

Every mutating subcommand is a **dry run** unless `-f, --force` is passed —
the same UX as `revert-ol-state`. Without `--force`, the command prints
what *would* happen and a `Use --force to execute these changes.` hint;
with `--force`, the mutation actually lands.

### Semantics — `abandon` vs `reset` vs `delete`

| Verb       | Final status                                    | When to use                                               |
|------------|-------------------------------------------------|-----------------------------------------------------------|
| `abandon`  | `PermanentFailure { error: "abandoned via dbtool" }` | Stop the recovery scanner from respawning a stuck task while keeping an audit trail. |
| `reset`    | `Pending` (retry-after cleared)                 | Force a fresh prove attempt — drops accumulated retry count. |
| `delete`   | row removed                                     | Prefer `abandon` unless you really want no trace left.    |
| `backfill` | `Pending` (newly inserted)                      | Queue a proof request "from outside" — e.g. for an epoch the node never picked up. |

### `get-prover-task`
Fetch a single prover task record by its hex-encoded key.

```bash
strata-dbtool get-prover-task <key_hex> [OPTIONS]
```

### `get-prover-tasks-summary`
Aggregate counts by status, plus a bounded slice of matching entries.

```bash
strata-dbtool get-prover-tasks-summary [--status <filter>] [--limit <n>] [OPTIONS]
```

**Options:**
- `--status <filter>` — one of `all` (default), `pending`, `proving`, `completed`, `transient-failure`, `permanent-failure`, `unfinished`, `terminal`
- `--limit <n>` — maximum entries to include in the output (default: 20)

### `abandon-prover-task`
Mark a single task as `PermanentFailure { error: "abandoned via dbtool" }`.

```bash
strata-dbtool abandon-prover-task <key_hex> --force
```

### `abandon-prover-tasks`
Bulk-abandon every `Pending` or `Proving` task. Without `--force`, prints
the change set as a dry run.

```bash
strata-dbtool abandon-prover-tasks --all-unfinished --force
```

### `reset-prover-task`
Flip a task back to `Pending` and clear its retry-after timestamp.

```bash
strata-dbtool reset-prover-task <key_hex> --force
```

### `delete-prover-task`
Hard-delete a task row.

```bash
strata-dbtool delete-prover-task <key_hex> --force
```

### `backfill-checkpoint-proof-task`
Queue a fresh `Pending` checkpoint-proof task for an epoch. Resolves the
canonical commitment at the epoch and constructs the task key via the shared
`CheckpointProofTask` encoding, so the running node will pick the task up on
its next startup-recovery pass.

```bash
strata-dbtool backfill-checkpoint-proof-task <epoch> --force
```

### `backfill-prover-task-raw`
Insert a `Pending` task record under a caller-provided raw key. Escape hatch
for proof kinds without a typed helper.

```bash
strata-dbtool backfill-prover-task-raw <key_hex> --force
```

### `get-checkpoint-proof`
Fetch the stored proof receipt for an OL checkpoint epoch.

```bash
strata-dbtool get-checkpoint-proof <epoch> [OPTIONS]
```

### `delete-checkpoint-proof`
Delete the stored proof receipt for an epoch. Operates on the canonical
commitment at that epoch. Use case: force a re-prove after a guest-program
upgrade.

```bash
strata-dbtool delete-checkpoint-proof <epoch> --force
```

## EE Prover Task & Receipt Admin

> [!WARNING]
>
> These commands mutate the EE prover store under the **alpen-client**
> datadir (not the strata node's). Stop the alpen-client before using
> them — concurrent writes from a running chunk/acct prover will conflict
> with these edits and may corrupt state.

The alpen-client maintains a separate sled instance for prover-side
persistence — shared task tree (chunk + acct), chunk-receipt store, and
typed acct-proof store. For every `ee-*` subcommand, point `-d`/`--datadir`
at the alpen-client's `--datadir` (the strata node's `-d` is never used
by these commands).

### Which surface to use

| Concern              | Lives in              | Subcommand prefix |
|----------------------|-----------------------|--------------------|
| OL checkpoint proofs | strata node datadir   | (no prefix)        |
| EE chunk proofs      | alpen-client datadir  | `ee-*` (`--kind chunk`) |
| EE acct/batch proofs | alpen-client datadir  | `ee-*` (`--kind acct`)  |

Chunk and acct tasks share one tree, disambiguated by a single-byte
**kind tag** at the start of every task key (`b'c'` for chunk, `b'a'`
for acct). The `--kind` filter on the summary and bulk-abandon commands
selects on that tag; single-key commands operate on opaque keys, so the
kind comes from whatever the key starts with.

### `ee-get-prover-task`
Fetch a single EE prover task record by its hex-encoded key.

```bash
strata-dbtool ee-get-prover-task <key_hex> [OPTIONS]
```

### `ee-get-prover-tasks-summary`
Aggregate counts by status, plus a bounded slice of matching entries.

```bash
strata-dbtool ee-get-prover-tasks-summary [--status <filter>] [--kind <kind>] [--limit <n>] [OPTIONS]
```

**Options:**
- `--status <filter>` — same set as the OL summary command (`all`, `pending`, …, `terminal`).
- `--kind <kind>` — one of `all` (default), `chunk`, `acct`.

### `ee-abandon-prover-task`
Mark a single EE task as `PermanentFailure { error: "abandoned via dbtool" }`.

```bash
strata-dbtool ee-abandon-prover-task <key_hex> --force
```

### `ee-abandon-prover-tasks`
Bulk-abandon every `Pending`/`Proving` EE task, optionally restricted by
kind. Without `--force`, prints the change set as a dry run.

```bash
strata-dbtool ee-abandon-prover-tasks --all-unfinished [--kind <kind>] --force
```

### `ee-reset-prover-task`
Flip an EE task back to `Pending` and clear its retry-after timestamp.

```bash
strata-dbtool ee-reset-prover-task <key_hex> --force
```

### `ee-delete-prover-task`
Hard-delete an EE task record.

```bash
strata-dbtool ee-delete-prover-task <key_hex> --force
```

### `ee-backfill-prover-task-raw`
Insert a `Pending` EE task record under a caller-provided raw key. EE
task keys come from the chunk/acct spec encodings; raw is the only
supported backfill path (no typed equivalent of `backfill-checkpoint-proof-task`).

```bash
strata-dbtool ee-backfill-prover-task-raw <key_hex> --force
```

### `ee-get-chunk-receipt` / `ee-delete-chunk-receipt`
Inspect, or delete-and-re-prove, a stored chunk-proof receipt by its task
key. Use case: drop a stale receipt after a guest-program upgrade so the
chunk prover re-proves it.

`ee-delete-chunk-receipt` removes **both** the chunk receipt and its
companion PaaS task record (they share the same key). Deleting the receipt
alone would not re-prove — re-proving is driven by the task store, and a
finished task is `Completed` (nothing re-runs it); it would instead wedge the
chunk in a `ProofReady`/`ProofPending` oscillation. Deleting the task record
lets the running node's acct gate flip the chunk back to `ProofPending` and
the chunk lifecycle resubmit it.

Run with the node down (like the other EE delete commands). The two-tree
delete is not transactional; re-running is safe (both deletes are
idempotent). If the batch's acct proof has already been produced, the acct
gate no longer re-fires for it — also run `ee-delete-acct-proof
<prev>:<last>` to force the chunk to re-prove. The output reports both
`existed` (receipt) and `task_existed`.

```bash
strata-dbtool ee-get-chunk-receipt <key_hex> [OPTIONS]
strata-dbtool ee-delete-chunk-receipt <key_hex> --force
```

### `ee-get-acct-proof` / `ee-delete-acct-proof`
Inspect or remove a stored acct/batch proof. The batch id is passed as
`<prev_block_hex>:<last_block_hex>` (each 32 bytes, optional `0x`
prefix), which is exactly what `BatchId::Display` emits — copy a
`%batch_id` field straight from the alpen-client's logs. Delete also
clears the secondary `ProofId → BatchId` index.

```bash
strata-dbtool ee-get-acct-proof <prev_block>:<last_block> [OPTIONS]
strata-dbtool ee-delete-acct-proof <prev_block>:<last_block> --force
```

### `ee-revert-batches`
Revert EE batch metadata from `--from-batch-idx` onward, delete the
corresponding unfinalized exec-block suffix, and clean exact affected
chunk/acct prover rows. Without `--force`, prints the full change set as
a dry run.

```bash
strata-dbtool -d <alpen-client-datadir> ee-revert-batches --from-batch-idx <n> [--tx-export <path>] [--force]
```

**Notes:**
- `--tx-export <path>` writes affected block transaction indexes, hashes,
  and raw 2718 bytes for manual rebroadcast. The file is written during
  dry-run too.
- The locally accepted frontier is derived from the EE sled, not from a live
  OL query. The command reads `best_ee_account_state()`, takes its
  `last_exec_blkid`, then scans stored EE batches until it finds the batch
  whose `last_block()` matches that execution block. The matching batch index
  is reported as `local_accepted_frontier.accepted_batch_idx`; the account
  state's `ol_epoch()` is reported as `local_accepted_frontier.best_epoch`.
  This is the last OL-accepted EE state observed by this EE node and may lag
  canonical OL if the snapshot is stale.
- If the rollback crosses the locally observed OL-accepted EE frontier,
  the command also rolls back EE account-state rows to the kept batch tip.
  If it cannot find a stored account-state epoch for that kept tip, the
  command blocks instead of leaving `best_ee_account_state` dangling.
- The report includes `affected_block_summary.reverted_batch` for blocks that
  belong to reverted sealed batches and `affected_block_summary.unbatched_suffix`
  for extra unbatched blocks built on top of the last reverted batch tip.
- Forced rollback is not one cross-tree transaction. The command rolls back
  EE account state when needed, deletes affected exec blocks and prover rows,
  then reverts batch metadata last. If a pre-final mutation fails, keep the node
  stopped and rerun the same command; earlier deletes are idempotent and the
  batch metadata is still available to reconstruct the plan. Export txs before
  forcing if manual rebroadcast data is needed after a partial retry.
- Old block-witness rows, chunk rows, and DA/broadcast rows are left in
  place; the report calls out these orphaned artifacts explicitly.

To inspect the rollback frontier during dry-run:

```bash
strata-dbtool -d <alpen-client-datadir> ee-revert-batches --from-batch-idx <n> -o json \
  | jq '.local_accepted_frontier'
```

## Output Formats

### Porcelain Format (Default)
Machine-readable, parseable format similar to `git --porcelain`. Each field is displayed as `key: value` pairs, one per line.

**Example:**
```
l1_tip.height: 800000
l1_tip.block_id: 42b3fd7680ea6141eec61ae5ae86e41163ab559b6a1ab86c4de9c540a2c5f63f
ol_tip.height: 1000
ol_tip.block_id: 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
ol_tip.block_status: Valid
```

### JSON Format
Structured JSON output for programmatic consumption.

**Example:**
```json
{
  "l1_tip_height": 800000,
  "l1_tip_block_id": "42b3fd7680ea6141eec61ae5ae86e41163ab559b6a1ab86c4de9c540a2c5f63f",
  "ol_tip_height": 1000,
  "ol_tip_block_id": "858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b"
}
```
