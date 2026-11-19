# Frozen V2 checkpoint program

This bundle is the V2 checkpoint program published from `v0.5.0-rc.2`. The SP1
upgrade tests use it twice: as the rotation target when a fresh V1 network
rotates to V2, and as the genesis program when a fresh V2 network rotates to
V3 built from the current checkout. Keeping the historical program fixed
checks that new host code can still prove V2 epochs on both sides of an
upgrade.

The ELF, predicate, and artifact manifest are copied unchanged from the
published bundle. The runtime-parameter file is copied unchanged from the
same workflow run. All four files are public protocol artifacts.

## Provenance

- Source: [`v0.5.0-rc.2`, commit `2853de3408bd5d865bb83844ac026a1aa3c379d5`](https://github.com/alpenlabs/alpen/tree/2853de3408bd5d865bb83844ac026a1aa3c379d5).
- Publisher: [GitHub Actions run 41777736368](https://github.com/alpenlabs/alpen/actions/runs/41777736368), SP1 `v6.8.1`.
- Bundle artifact: `sp1-artifacts-2853de34-fbbfce56`.
- Runtime-parameter artifact: `sp1-runtime-params`.
- S3 destination recorded in the run log: `s3://alpen-mosaic-artifacts/elfs/alpen/2853de34-fbbfce56/`.

The artifact manifest records spec `2`, program ID
`00c8f06d2991fde68254e2306383dd14e2725dcca883d4bf968a7b7e3e1e1f40`,
and runtime-parameter hash
`9c298590a7a9f4d9c06fd5a0e43547ff84e90f70ae816e876f738e900542bb5e`.
That runtime hash is the protocol hash, distinct from the JSON file checksum
below. The node checks the manifest against its runtime parameters and the
program ID and predicate derived from the ELF when it loads the bundle.

## Runtime parameters

The adjacent `checkpoint-runtime-params.json` is byte-identical to the
[frozen V1 bundle's](../v1/README.md): a bridge denomination of `200000000`
sats, no maximum withdrawal amount (`null`), and a maximum withdrawal
descriptor length of `81`. Both manifests record the same runtime-parameter
hash. The SP1 upgrade tests and the V3 guest build must use these values.
They differ from the native test defaults, so the SP1 path selects them
explicitly.

Keep this V2 bundle fixed when rebuilding the current guest. Changing the
parameters requires matching historical V1 and V2 builds and predicates;
editing the manifest does not change the parameters embedded in the ELF.

## Checksums and retrieval

All four checksums were verified against the publication metadata and runtime
artifact. From this directory, verify the files with:

```sh
sha256sum --check <<'SHA256'
f9d453b7ed6d7f524901adcf1010cb4c0d08922158553510b722f80ce5e41ba3  guest-checkpoint.elf
26b731023874640a526361bd047b95f3135760da08e94c02a9abe7fa74fc6b06  guest-checkpoint.predicate
6a5fa8b97f01b05f1af54f6b24988341f2114253c87b676c3e63050e0d98f789  guest-checkpoint.artifact-manifest.json
fbbfce568f235abeeeddc4879fb9dfbe4cf5d7c295d198c3f2169861e6f550cb  checkpoint-runtime-params.json
SHA256
```

While GitHub retains the artifacts, retrieve the originals into temporary
folders from the repository root:

```sh
gh run download 41777736368 --repo alpenlabs/alpen \
  --name sp1-artifacts-2853de34-fbbfce56 --dir /tmp/alpen-v2-bundle
gh run download 41777736368 --repo alpenlabs/alpen \
  --name sp1-runtime-params --dir /tmp/alpen-v2-runtime
```

The committed files let future test runs use the frozen bundle after Actions
artifacts expire, without depending on AWS access.
