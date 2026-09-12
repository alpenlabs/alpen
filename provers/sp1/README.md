# SP1 guest builder

Compiles the SP1 guest programs in this directory and exports their artifacts to `generated/`.

Each guest lives in its own `guest-*` directory with its own Cargo workspace, so it can pin the
SP1 patches it needs without disturbing the main workspace.

## Building

Both steps are opt-in, because both are slow and most builds only need this crate to compile:

```bash
# Compile the guest ELFs.
BUILD_ELF=1 cargo build --release -p strata-sp1-guest-builder

# Also derive the verifying keys and write the `*.vk-hash` and `*.predicate` files.
BUILD_ELF=1 BUILD_VKEY=1 cargo build --release -p strata-sp1-guest-builder
```

Add `--features docker-build` to compile the guests inside a pinned Docker image, which is what
the artifact publishing workflow uses for reproducibility.

## Artifacts

`generated/` is gitignored and survives `cargo clean`, so a build that skips the steps above leaves
whatever was built earlier in place.

| File | Contents |
|------|----------|
| `<guest>.elf` | The compiled guest program |
| `<guest>.vk-hash` | The verifying key's `bytes32` program ID |
| `<guest>.predicate` | `Sp1Groth16:<hex>`, read by `strata-datatool` when building params |
