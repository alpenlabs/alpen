# ASM Spec Configuration

This crate implements the Anchor State Machine (ASM) specification for the Strata protocol with compile-time configuration support.

## Configuration-Driven Build

The ASM spec uses a build script to read configuration from JSON files at compile time, eliminating hardcoded values and providing environment-specific configurations.

### Configuration Files

- `asm_config.json` - Default/development configuration
- `asm_config.testnet.json` - Testnet configuration  
- `asm_config.mainnet.json` - Mainnet configuration (create as needed)

### Configuration Structure

```json
{
  "magic_bytes": "ALPN",
  "core_genesis": {
    "checkpoint_vk_file": "../../../crates/test-utils/data/sp1_rollup_vk.json",
    "genesis_l1_block": {
      "height": 100,
      "block_id": "0000000000000000000000000000000000000000000000000000000000000000"
    },
    "sequencer_pubkey": "76849911e8c3bb3d55c9f9cedec8f9e5621fcc4aa791bd1f10369ee435b56b1f"
  },
  "bridge_genesis": {
    "note": "Bridge-specific parameters will be added here"
  }
}
```

### Building with Different Configurations

**Default (development):**
```bash
cargo build -p strata-asm-spec
```

**Testnet:**
```bash
ASM_CONFIG_ENV=testnet cargo build -p strata-asm-spec
```

**Mainnet:**
```bash
ASM_CONFIG_ENV=mainnet cargo build -p strata-asm-spec
```

### Generated Constants

The build script generates the following constants and functions:

- `MAGIC_BYTES: [u8; 4]` - Protocol magic bytes
- `core_genesis::checkpoint_vk()` - Checkpoint verifying key
- `core_genesis::genesis_l1_block()` - Genesis L1 block commitment
- `core_genesis::sequencer_pubkey()` - Authorized sequencer public key

### Type Safety

All configuration values are validated at build time:

- Magic bytes must be exactly 4 characters
- Block IDs and public keys must be valid 64-character hex strings
- Referenced files (e.g., verifying key) must exist

Invalid configurations will cause compilation to fail with clear error messages.

### Adding New Parameters

1. Add the parameter to the configuration JSON structure
2. Update the `AsmConfig` struct in `build.rs`
3. Add validation in `validate_config()`
4. Add generation logic in `generate_rust_code()`
5. Use the generated constants in your implementation

### Testing

The crate includes tests that verify configuration loading:

```bash
# Test default config
cargo test -p strata-asm-spec

# Test testnet config  
ASM_CONFIG_ENV=testnet cargo test -p strata-asm-spec
```

Tests verify that different configurations produce different magic bytes and genesis parameters as expected.