# SP1 guest builder

Compiles the SP1 guest programs in this directory and exports their artifacts to `generated/`.

Each guest lives in its own `guest-*` directory with its own Cargo workspace, so it can pin the
SP1 patches it needs without disturbing the main workspace.

## Building

Both steps are opt-in, because both are slow and most builds only need this crate to compile.
`CHECKPOINT_RUNTIME_PARAMS_PATH` is required whenever a guest is actually built, because the OL
runtime params are compiled into the checkpoint guest and are part of what it proves:

```bash
# Compile the guest ELFs.
BUILD_ELF=1 \
CHECKPOINT_RUNTIME_PARAMS_PATH=/path/to/ol-params.json \
    cargo build --release -p strata-sp1-guest-builder

# Also derive the verifying keys and write the `*.vk-hash`, `*.predicate` and
# `*.artifact-manifest.json` files.
BUILD_ELF=1 BUILD_VKEY=1 \
CHECKPOINT_RUNTIME_PARAMS_PATH=/path/to/ol-params.json \
    cargo build --release -p strata-sp1-guest-builder
```

Add `--features docker-build` to compile the guests inside a pinned Docker image, which is what
the artifact publishing workflow uses for reproducibility.

### The runtime params file

Point `CHECKPOINT_RUNTIME_PARAMS_PATH` at the same `ol-params.json` the node will load. The build
accepts either that full params document or a bare runtime-params one, so the file needs no
trimming. Generate it with `strata-datatool gen-ol-params` — see
[`bin/datatool/README.md`](../../bin/datatool/README.md). It does not depend on any SP1 artifact,
so it can always be built first; only `gen-asm-params` needs the predicate this crate produces.

For a local network, `docker/gen-params-and-elfs.sh` already wires this up: it generates
`ol-params.json`, builds the guests against it, then generates the ASM params from the resulting
predicate.

A node checks the params it loaded against the ones baked into the ELF at startup, so a mismatch
here surfaces as a startup error rather than as proofs over the wrong rules.

## Artifacts

`generated/` is gitignored and survives `cargo clean`, so a build that skips the steps above leaves
whatever was built earlier in place.

| File | Contents |
|------|----------|
| `<guest>.elf` | The compiled guest program |
| `<guest>.vk-hash` | The verifying key's `bytes32` program ID |
| `<guest>.predicate` | `Sp1Groth16:<hex>`, read by `strata-datatool` when building params |
| `<guest>.artifact-manifest.json` | The program ID and the hash of the runtime params baked into the ELF, checked at node startup |
