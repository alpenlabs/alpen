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
| `<guest>.artifact-manifest.json` | Schema version, proved OL spec, program ID, and hash of the runtime params baked into the ELF |

The node checks the manifest's program ID and runtime-params hash at startup. The `spec` field
records the rules compiled into the guest; it can differ from the network's `genesis.spec`.

## Publishing

The [Publish SP1 Artifacts workflow](../../.github/workflows/publish-sp1-artifacts.yml) requires
`env` (`dev`, `staging`, `testnet`, or `mainnet`) and `checkpoint_runtime_params_url` for manual
dispatch and reusable workflow calls. Run the workflow from `main` and set `ref` to the commit,
tag, or branch to build (default: `main`). Publishing scripts and local actions stay on the
workflow's `main` commit; the guest builds in a separate checkout of `ref`, using its Rust
toolchain and locked SP1 build and runner versions. The selected ref must support the builder
and artifact bundle described above.

Supply a raw HTTPS download URL for either params format described above. GitHub `/blob/`
URLs serve HTML and are rejected. The workflow downloads and JSON-validates the file in
`${{ runner.temp }}`. The preflight job resolves the source commit and checks S3 before
toolchain setup or compilation. If a complete bundle already exists and its manifest matches
the full commit SHA and params checksum, the workflow reports its location and skips building
and publishing. Partial bundles, mismatched manifests, and AWS errors stop the workflow early.
For a new bundle, the build checks out the resolved SHA and receives the same downloaded params
through a workflow artifact, passing their path through `CHECKPOINT_RUNTIME_PARAMS_PATH`.

The bundle includes all four artifacts above, their SHA-256 sidecars, and a separate
`manifest.json` recording the built ref and commit, requesting network, SP1 version, params URL,
and file checksums.
The downloaded JSON's checksum records the input bytes; `runtime_params_hash` in the guest
manifest hashes the SSZ-encoded runtime params.

The preflight and publish jobs authenticate through GitHub OIDC using the existing S3 role and
the `sp1-artifacts` environment. Preflight requires `s3:ListBucket` and `s3:GetObject`; the build
job receives no AWS credentials. The publish job checks every required file and checksum before
uploading to `s3://alpen-mosaic-artifacts/elfs/alpen/<commit8>-<params8>/`, where `commit8` and `params8` are
the first eight characters of the built commit SHA and downloaded JSON's SHA-256. This
write-once location is shared across networks: reuse the existing bundle for the same commit
and exact params file instead of publishing it again for each network. `env` records the
network requesting the original publish and does not restrict which networks can use it.

After a failed upload, check the bundle prefix for `manifest.json`, which is uploaded last.
If it and all bundle files exist and are nonempty, reuse the completed bundle. Otherwise, once
no publish for this bundle is running, have an operator remove the incomplete prefix. If the
publish job failed, use **Re-run failed jobs** to retry with the same workflow artifacts;
if preflight failed, rerun it after cleanup. Existing objects prevent retries from succeeding
until the incomplete prefix is removed.
