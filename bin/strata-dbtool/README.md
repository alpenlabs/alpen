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

- `-d, --datadir <path>` - Node data directory (default: `data`)
- `-t, --db-type <type>` - Backend DB implementation: `sled` (default: `sled`)
- `-o, --output-format <format>` - Output format: `porcelain` (default) or `json`

## Commands

### `get-syncinfo`
Shows the latest synchronization information including L1/L2 tips, epochs, and block status.

```bash
strata-dbtool get-syncinfo [OPTIONS]
```

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-syncinfo
```

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
strata-dbtool get-l1-summary [<height_from>] [OPTIONS]
```

**Arguments:**
- `height_from` - L1 height to look up the summary about

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

### `get-l1-manifest`
Shows detailed information about a specific L1 block manifest.

```bash
strata-dbtool get-l1-manifest <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - L1 block ID (hex string)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-l1-manifest 42b3fd7680ea6141eec61ae5ae86e41163ab559b6a1ab86c4de9c540a2c5f63f
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

### `get-l2-summary`
Shows a summary of L2 blocks in the database.

```bash
strata-dbtool get-l2-summary [OPTIONS]
```

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

### `get-l2-block`
Shows detailed information about a specific L2 block.

```bash
strata-dbtool get-l2-block <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - L2 block ID (hex string)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-l2-block 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
```

### `get-checkpoints-summary`
Shows a summary of all checkpoints in the database.

```bash
strata-dbtool get-checkpoints-summary <height_from>  [OPTIONS]
```

**Arguments:**
- `height_from` - Start l1 height to query checkpoints from

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-checkpoints-summary 10
```

### `get-checkpoint`
Shows detailed information about a specific checkpoint.

```bash
strata-dbtool get-checkpoint <checkpoint_index> [OPTIONS]
```

**Arguments:**
- `checkpoint_index` - The checkpoint index (number)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-checkpoint 5
```

### `get-epoch-summary`
Shows detailed information about a specific epoch.

```bash
strata-dbtool get-epoch-summary <epoch_index> [OPTIONS]
```

**Arguments:**
- `epoch_index` - The epoch index (number)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-epoch-summary 5
```

### `get-chainstate`
Shows the current chain state information.

```bash
strata-dbtool get-chainstate <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - L2 block ID (hex string)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-chainstate 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
```

### `revert-chainstate`
Reverts the chain state to a specific block ID.

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
strata-dbtool revert-chainstate <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - Target L2 block ID to revert to (hex string)

**Options:**
- `-f, --force` - Force execution (without this flag, only a dry run is performed)
- `-d, --delete-blocks` - Delete blocks after target block (not just mark as unchecked)
- `-c, --revert-checkpointed-blocks` - Allow reverting blocks inside checkpointed epoch

**Examples:**

Dry run (default behavior - shows what would be affected):
```bash
strata-dbtool revert-chainstate 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
```

Actually execute the revert:
```bash
strata-dbtool revert-chainstate --force 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
```

Execute revert with block deletion:
```bash
strata-dbtool revert-chainstate -f -d 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
```

## Output Formats

### Porcelain Format (Default)
Machine-readable, parseable format similar to `git --porcelain`. Each field is displayed as `key=value` pairs, one per line.

**Example:**
```
l1_tip_height=800000
l1_tip_block_id=42b3fd7680ea6141eec61ae5ae86e41163ab559b6a1ab86c4de9c540a2c5f63f
l2_tip_height=1000
l2_tip_block_id=858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
```

### JSON Format
Structured JSON output for programmatic consumption.

**Example:**
```json
{
  "l1_tip_height": 800000,
  "l1_tip_block_id": "42b3fd7680ea6141eec61ae5ae86e41163ab559b6a1ab86c4de9c540a2c5f63f",
  "l2_tip_height": 1000,
  "l2_tip_block_id": "858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b"
}
```