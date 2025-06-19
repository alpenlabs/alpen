# Alpen CLI Protocol Parameters

This directory contains configuration files and examples for alpen-cli.

## Overview

Alpen CLI now supports configurable protocol parameters, allowing you to customize bridge behavior without recompiling. Previously hardcoded constants can now be adjusted per network or deployment.

## Quick Start

### Generate Default Parameters
```bash
# Generate default params file
alpen generate-params

# Generate to custom location
alpen generate-params --output my-params.json
```

### Use Custom Parameters
```bash
# Via command line flag
alpen --protocol-params my-params.json deposit

# Via environment variable
export ALPEN_CLI_PARAMS=@/path/to/params.json
alpen deposit

# Via default location
cp my-params.json ~/.config/alpen/params.json
alpen deposit  # automatically uses params.json
```

## Parameters Reference

### `bridge_recover_delay`
**Default**: 1008 blocks (~1 week)
**Description**: Number of blocks to wait before recovery path becomes spendable
**Impact**: Security vs convenience tradeoff for fund recovery

### `bridge_finality_depth` 
**Default**: 6 blocks
**Description**: Number of blocks to consider a transaction final (reorg protection)
**Impact**: Affects when recovery process can begin

### `bridge_in_amount`
**Default**: 1,000,001,000 satoshis (10 BTC + 1k sats fee buffer)
**Description**: Fixed deposit amount including fee buffer
**Impact**: How much BTC is locked in each bridge deposit

### `bridge_out_amount`
**Default**: 1,000,000,000 satoshis (10 BTC exact)
**Description**: Fixed withdrawal amount
**Impact**: How much BTC is returned in each bridge withdrawal

### `recovery_desc_cleanup_delay`
**Default**: 100 blocks
**Description**: How long to keep recovery descriptors after they're no longer needed
**Impact**: Disk usage and recovery reliability

## Configuration Hierarchy

Alpen CLI loads parameters in this order (highest to lowest priority):

1. **Explicit path**: `--protocol-params /path/to/params.json`
2. **Environment variable**: `ALPEN_CLI_PARAMS=@/path/to/params.json` (file) or `ALPEN_CLI_PARAMS='{"bridge_recover_delay":100,...}'` (inline JSON)
3. **Default location**: `~/.config/alpen/params.json`
4. **Built-in defaults**: Single set of sensible defaults

## Default Parameters

All networks use the same sensible defaults that you can customize:

```json
{
  "bridge_recover_delay": 1008,
  "bridge_finality_depth": 6,
  "bridge_in_amount": 1000001000,
  "bridge_out_amount": 1000000000,
  "recovery_desc_cleanup_delay": 100
}
```

## Network-Specific Customizations

You may want to adjust parameters based on your network:

### Mainnet (Security-focused)
```json
{
  "bridge_recover_delay": 2016  // ~2 weeks for extra security
}
```

### Regtest (Development)
```json
{
  "bridge_recover_delay": 144,     // ~1 day for faster testing
  "bridge_finality_depth": 2,      // Faster confirmation
  "recovery_desc_cleanup_delay": 10 // Quick cleanup
}
```

## Validation

All parameters are validated when loaded:
- Recovery delay must be > 0
- Finality depth must be > 0  
- Bridge out amount must be ≤ bridge in amount
- All values must be reasonable for the network

## Examples

### Development Setup
```bash
# Fast recovery for testing
alpen generate-params --network regtest --output dev-params.json
alpen --protocol-params dev-params.json deposit

# Or copy the example config
cp src/config/params.json my-params.json
# Edit my-params.json as needed
alpen --protocol-params my-params.json deposit
```

### Production Setup
```bash
# Conservative settings for mainnet
alpen generate-params --network mainnet
# Edit ~/.config/alpen/params.json to adjust recovery_delay if needed
alpen deposit  # uses default params location

# Or use the example as a starting point
cp src/config/params.json ~/.config/alpen/params.json
# Edit ~/.config/alpen/params.json as needed
```

### Custom Recovery Period
```bash
# Create params with 3-day recovery
echo '{
  "bridge_recover_delay": 432,
  "bridge_finality_depth": 6, 
  "bridge_in_amount": 1000001000,
  "bridge_out_amount": 1000000000,
  "recovery_desc_cleanup_delay": 100
}' > custom-params.json

alpen --protocol-params custom-params.json deposit
```