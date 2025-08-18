# Checkpoint Validator

A command-line tool for validating Bitcoin transaction checkpoints against rollup parameters in the Strata blockchain system.

## Overview

This tool takes a raw Bitcoin transaction (in hex format) and validates any checkpoints found within it against the provided rollup parameters configuration. It uses the existing Strata checkpoint validation logic to ensure checkpoints are properly formatted, have valid proofs, and follow the correct sequencing rules.

## Usage

```bash
checkpoint-validator [--tx <hex_tx> | --tx-file <file>] --config <rollup_params.json> [options]
```

### Required Arguments

- Transaction input (exactly one of):
  - `--tx`, `-t`: Raw Bitcoin transaction in hex format
  - `--tx-file`: Path to file containing raw Bitcoin transaction in hex format
- `--config`, `-c`: Path to rollup parameters configuration file (JSON format)

### Optional Arguments

- `--prev-checkpoint`, `-p`: Path to previous checkpoint file for proper validation (JSON format)
- `--format`, `-f`: Output format, either `human` (default) or `json`
- `--verbose`, `-v`: Enable verbose logging
- `--help`: Show help message

## Examples

### Basic Usage

```bash
# Validate a transaction with hex from command line
checkpoint-validator \
  --tx "02000000000101..." \
  --config rollup_params.json

# Validate a transaction with hex from file (useful for large transactions)
checkpoint-validator \
  --tx-file transaction.hex \
  --config rollup_params.json
```

### With Previous Checkpoint Context

```bash
# Validate with previous checkpoint for sequence validation
checkpoint-validator \
  --tx-file transaction.hex \
  --config rollup_params.json \
  --prev-checkpoint previous_checkpoint.json
```

### JSON Output

```bash
# Get machine-readable JSON output
checkpoint-validator \
  --tx-file transaction.hex \
  --config rollup_params.json \
  --format json
```

### Verbose Logging

```bash
# Enable detailed logging for debugging
checkpoint-validator \
  --tx-file transaction.hex \
  --config rollup_params.json \
  --verbose
```

### Large Transaction Files

```bash
# For very large transactions with multiple checkpoints
echo "020000000001..." > large_transaction.hex
checkpoint-validator \
  --tx-file large_transaction.hex \
  --config rollup_params.json \
  --verbose \
  --format json > validation_results.json
```

## File Formats

### Transaction File Format

Transaction files should contain the raw Bitcoin transaction in hexadecimal format. The file format is flexible:

```bash
# Single line (recommended for large transactions)
020000000001017abc123...

# Multiple lines (will be joined automatically)
020000000001017abc123def456...
789ghi012jkl345mno678...
pqr901stu234vwx567yz0...

# With comments (lines starting with # are ignored)
# Large transaction with multiple checkpoints
# Transaction ID: abc123def456...
020000000001017abc123def456...
789ghi012jkl345mno678...
```

**Note:** Whitespace, newlines, and lines starting with `#` are automatically removed during parsing.

## Configuration Files

### Rollup Parameters (rollup_params.json)

The rollup parameters file should contain the rollup configuration in JSON format. Example:

```json
{
  "rollup_name": "ALPN",
  "block_time": 1000,
  "da_tag": "strata-da",
  "checkpoint_tag": "strata-ckpt",
  "cred_rule": {
    "schnorr_key": "7c9a485ad5e94a1454e605c63403538abe59a17657b033c8b7d23223a5062802"
  },
  "horizon_l1_height": 90,
  "genesis_l1_height": 100,
  "rollup_vk": "native",
  "proof_publish_mode": "strict",
  "network": "signet"
}
```

### Previous Checkpoint (optional)

If provided, the previous checkpoint file should contain the L1 checkpoint data in JSON format for proper sequence validation.

## Output Formats

### Human-Readable Output (default)

```
=== Checkpoint Validation Results ===
Checkpoints found: 2
Checkpoints validated: 2
Checkpoints failed: 0
Overall success: true

Individual Results:
  Epoch 42: ✓ VALID
  Epoch 43: ✓ VALID
```

### JSON Output

```json
{
  "success": true,
  "checkpoints_found": 2,
  "checkpoints_validated": 2,
  "checkpoints_failed": 0,
  "results": [
    {
      "epoch": 42,
      "valid": true,
      "error": null
    },
    {
      "epoch": 43,
      "valid": true,
      "error": null
    }
  ],
  "errors": []
}
```

## Exit Codes

- `0`: All checkpoints validated successfully
- `1`: One or more checkpoints failed validation or other error occurred

## How It Works

1. **Transaction Parsing**: Parses the hex-encoded Bitcoin transaction
2. **Configuration Loading**: Loads and validates the rollup parameters
3. **Checkpoint Extraction**: Uses Strata's envelope parsing logic to extract checkpoints from transaction inputs
4. **Validation**: Validates each checkpoint using Strata's consensus logic:
   - Proof verification (if not in empty proof mode)
   - State transition validation
   - Sequence validation (if previous checkpoint provided)
   - Signature verification
5. **Results**: Outputs detailed validation results

## Building

To build the checkpoint validator:

```bash
# From the workspace root
cargo build -p checkpoint-validator

# Or build in release mode
cargo build --release -p checkpoint-validator
```

The binary will be available at:
- Debug: `target/debug/checkpoint-validator`
- Release: `target/release/checkpoint-validator`

## Dependencies

This tool uses the following Strata crates:
- `strata-primitives`: For rollup parameters and basic types
- `strata-l1tx`: For parsing checkpoints from Bitcoin transactions
- `strata-consensus-logic`: For checkpoint validation logic
- `strata-state`: For checkpoint and state types

## Error Handling

The tool provides detailed error messages for various failure conditions:
- Invalid hex transaction format
- Invalid rollup parameters
- Transaction parsing failures
- Checkpoint validation failures
- File I/O errors

All errors are logged with appropriate context to help with debugging.