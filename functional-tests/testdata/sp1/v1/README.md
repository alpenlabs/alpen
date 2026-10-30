# Frozen V1 checkpoint program

This bundle is the V1 checkpoint program deployed on staging-v2. The SP1
upgrade test will start a fresh chain with this program, then rotate to V2
built from the current checkout. Keeping the historical program fixed checks
that new host code can still prove V1 epochs across the upgrade.

The ELF, predicate, and artifact manifest are copied unchanged from the
published bundle. The runtime-parameter file is copied unchanged from the
same workflow run. All four files are public protocol artifacts.

## Provenance

- Source: [`v0.4.0-rc.1`, commit `d65b7cc8945437c7f23f1c4340d55d367989d6d1`](https://github.com/alpenlabs/alpen/tree/d65b7cc8945437c7f23f1c4340d55d367989d6d1).
- Publisher: [GitHub Actions run 37644982166](https://github.com/alpenlabs/alpen/actions/runs/37644982166), SP1 `v6.8.1`.
- Bundle artifact: `sp1-artifacts-d65b7cc8-fbbfce56`.
- Runtime-parameter artifact: `sp1-runtime-params`.
- S3 source: `s3://alpen-mosaic-artifacts/elfs/alpen/d65b7cc8-fbbfce56/`.
- Checked on 2026-10-30: the running staging-v2 `rollup/strata-ol-0` pod's
  three bundle files have these same SHA-256 hashes, and its mounted prover
  configuration selects `spec = 1` at `/app/elfs/sp1/v1`.

The artifact manifest records spec `1`, program ID
`00cfee215d1adb8bd536afa176806803b038e3f0fb473e98becf39dd32ed5372`,
and runtime-parameter hash
`9c298590a7a9f4d9c06fd5a0e43547ff84e90f70ae816e876f738e900542bb5e`.
That runtime hash is the protocol hash, distinct from the JSON file checksum
below. The node checks the manifest against its runtime parameters and the
program ID and predicate derived from the ELF when it loads the bundle.

## Runtime parameters

The program embeds a bridge denomination of `200000000` sats, no maximum
withdrawal amount (`null`), and a maximum withdrawal descriptor length of
`81`. The SP1 upgrade test and its V2 guest build must both use the adjacent
`checkpoint-runtime-params.json`. These values differ from the native test
defaults, so the SP1 path must select them explicitly.

Keep this V1 bundle fixed when rebuilding V2. Changing the parameters requires
another matching historical V1 build and predicate; editing the manifest does
not change the parameters embedded in the ELF.

## Checksums and retrieval

All four checksums were verified against the publication metadata and runtime
artifact. From this directory, verify the files with:

```sh
sha256sum --check <<'SHA256'
93a5328fac1581ec4ec3eac369140439384b7b75e754ab46535bcb72ba086fe8  guest-checkpoint.elf
1dff48eba37e86dc63cdc932df2035f152921ced144fd9bac9c93b3b7ceec318  guest-checkpoint.predicate
b697208e67b5af30b0e3b963843b431c69c4e1f75fdfb8f9e1249bb5c2711933  guest-checkpoint.artifact-manifest.json
fbbfce568f235abeeeddc4879fb9dfbe4cf5d7c295d198c3f2169861e6f550cb  checkpoint-runtime-params.json
SHA256
```

While GitHub retains the artifacts, retrieve the originals into temporary
folders from the repository root:

```sh
gh run download 37644982166 --repo alpenlabs/alpen \
  --name sp1-artifacts-d65b7cc8-fbbfce56 --dir /tmp/alpen-v1-bundle
gh run download 37644982166 --repo alpenlabs/alpen \
  --name sp1-runtime-params --dir /tmp/alpen-v1-runtime
```

The committed files let future test runs use the frozen bundle after Actions
artifacts expire, without depending on AWS access.
