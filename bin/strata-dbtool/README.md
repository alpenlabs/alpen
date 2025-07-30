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
- `-t, --db-type <type>` - Backend DB implementation: `rocksdb` or `sled` (default: `rocksdb`)
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
Shows the latest client state update information.

```bash
strata-dbtool get-client-state-update [<update_index>] [OPTIONS]
```
**Arguments:**
- `update_index` - Client state update index (number), defaults to the latest

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

### `get-l1-summary`
Shows a summary of all L1 manifests in the database.

```bash
strata-dbtool get-l1-summary [OPTIONS]
```

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
strata-dbtool get-checkpoints-summary [OPTIONS]
```

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

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

### `get-sync-events-summary`
Shows a summary of all sync events in the database.

```bash
strata-dbtool get-sync-events-summary [OPTIONS]
```

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

### `get-sync-event`
Shows sync event information for a specific event index.

```bash
strata-dbtool get-sync-event <event_index> [OPTIONS]
```

**Arguments:**
- `event_index` - sync event index (number)

**Options:**
- `-o, --output-format <format>` - Output format (default: porcelain)

**Example:**
```bash
strata-dbtool get-sync-event 80
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
Reverts the chain state to a specific block ID. **Use with caution!**

```bash
strata-dbtool revert-chainstate <block_id> [OPTIONS]
```

**Arguments:**
- `block_id` - Target L2 block ID to revert to (hex string)

**Options:**
- `-d, --delete-blocks` - delete blocks after target block
- `-c, --revert-checkpointed-blocks` - allow reverting blocks inside checkpointed epoch

**Example:**
```bash
strata-dbtool revert-chainstate 858c390aaaabd7c457cb24c955d06fb9de0f6666d0b692e3b1a01b426705885b
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
