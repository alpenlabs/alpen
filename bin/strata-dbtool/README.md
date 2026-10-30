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

- `-d, --datadir <path>` - Data directory of the node whose DB is being inspected (default: `data`).

## Commands

### `get-syncinfo`
Shows the latest synchronization information including L1/OL tips, epochs, and block status.

```bash
strata-dbtool get-syncinfo --node-role sequencer [OPTIONS]
```

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).
- `-o, --output-format <format>` - Output format (default: porcelain)
- `--l1-reorg-safe-depth <depth>` - L1 reorg-safe depth used to derive finalized checkpoint epoch

**Example:**
```bash
strata-dbtool get-syncinfo --node-role sequencer --l1-reorg-safe-depth 6
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
strata-dbtool get-writer-summary --node-role sequencer [OPTIONS]
```

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-writer-summary --node-role sequencer
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
strata-dbtool get-checkpoints-summary --node-role sequencer <height_from> [OPTIONS]
```

**Arguments:**
- `height_from` - Start L1 height to query checkpoints from

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-checkpoints-summary --node-role sequencer 10
```

### `get-checkpoint`
Shows detailed information about a specific OL checkpoint epoch.

```bash
strata-dbtool get-checkpoint --node-role sequencer <checkpoint_epoch> [OPTIONS]
```

**Arguments:**
- `checkpoint_epoch` - Checkpoint epoch (number)

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).
- `-o, --output-format <format>` - Output format (default: porcelain)
- `--l1-reorg-safe-depth <depth>` - L1 reorg-safe depth used to derive checkpoint status

**Example:**
```bash
strata-dbtool get-checkpoint --node-role sequencer 5 --l1-reorg-safe-depth 6
```

**Notes:**
- Checkpoint status is reported as canonical `checkpoint.status`:
  `Unsigned`, `Signed`, `Confirmed`, or `Finalized`.
- For `Signed`, output includes `checkpoint.intent_index`.

### `get-epoch-summary`
Shows detailed information about a specific OL epoch summary.

```bash
strata-dbtool get-epoch-summary --node-role sequencer <epoch> [OPTIONS]
```

**Arguments:**
- `epoch` - epoch

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-epoch-summary --node-role sequencer 5
```

### `get-ol-state`
Shows the current OL state information.

```bash
strata-dbtool get-ol-state --node-role sequencer <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - OL block ID (hex string)

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).
- `-o, --output-format <format>` - Output format (default: porcelain)
- `--l1-reorg-safe-depth <depth>` - L1 reorg-safe depth used to derive finalized checkpoint epoch

**Example:**
```bash
strata-dbtool get-ol-state --node-role sequencer 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b --l1-reorg-safe-depth 6
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
strata-dbtool revert-ol-state --node-role sequencer <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - Target OL block ID to revert to (hex string)

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).
- `-f, --force` - Force execution (without this flag, only a dry run is performed)
- `-d, --delete-blocks` - Delete blocks after target block (not just mark as unchecked)
- `-c, --revert-checkpointed-blocks` - Allow reverting blocks inside checkpointed epoch
- `--l1-reorg-safe-depth <depth>` - L1 reorg-safe depth used to derive finalized checkpoint epoch for revert safety checks

**Examples:**

Dry run (default behavior - shows what would be affected):
```bash
strata-dbtool revert-ol-state --node-role sequencer 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b --l1-reorg-safe-depth 6
```

Actually execute the revert:
```bash
strata-dbtool revert-ol-state --node-role sequencer --force 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b --l1-reorg-safe-depth 6
```

Execute revert with block deletion:
```bash
strata-dbtool revert-ol-state --node-role sequencer -f -d 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b --l1-reorg-safe-depth 6
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
Flip a task back to `Pending` and clear its retry-after timestamp. Retains task
metadata, including any saved remote request ID, so this can resume the same
remote job rather than create a new one.

```bash
strata-dbtool reset-prover-task <key_hex> --force
```

### `delete-prover-task`
Delete a task row and its metadata, including any saved remote request ID.
A later submission for the same key can create a fresh task. This does not delete
its proof receipt or checkpoint payload/signing records.

```bash
strata-dbtool delete-prover-task <key_hex> --force
```

### `backfill-checkpoint-proof-task`
Queue a fresh `Pending` checkpoint-proof task for an epoch. Resolves the
canonical commitment at the epoch and reads its terminal OL state's current
spec to prefix the task key through `VersionedTaskStore`. A prover service for
that spec can recover the task on startup. Missing or unreadable terminal state
causes an error without inserting a task, including during a dry run. Without
`--force`, the command only previews the task; existing task records are never
overwritten. This command does not add proving support for a spec or migrate
existing tasks.

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).

```bash
strata-dbtool backfill-checkpoint-proof-task --node-role sequencer <epoch> --force
```

### `backfill-prover-task-raw`
Insert a `Pending` task record under a caller-provided raw key. Escape hatch
for proof kinds without a typed helper.

```bash
strata-dbtool backfill-prover-task-raw <key_hex> --force
```

### `get-checkpoint-proof`
Fetch the stored proof receipt for an OL checkpoint epoch.

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).

```bash
strata-dbtool get-checkpoint-proof --node-role sequencer <epoch> [OPTIONS]
```

### `delete-checkpoint-proof`
Delete the stored proof receipt for an epoch's canonical commitment. This leaves
the prover task and checkpoint payload/signing records unchanged. Deleting the
receipt alone does not force reproving: a completed task still needs to be reset
with `reset-prover-task` or removed with `delete-prover-task`. Those commands do
not make the checkpoint runner revisit an epoch it has already passed.

There is no dedicated command to remove only checkpoint payload/signing records.
Reverting checkpointed history with `revert-ol-state` also removes epoch summaries
and rewinds OL state. Replacing a guest artifact under the same spec is not a
supported protocol upgrade; see
[checkpoint work across restarts](../../crates/ol/checkpoint-artifacts/README.md#checkpoint-work-across-restarts).

**Options:**
- `--node-role <role>` - Effective node role: `sequencer` or `checkpoint-sync` (required).

```bash
strata-dbtool delete-checkpoint-proof --node-role sequencer <epoch> --force
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
