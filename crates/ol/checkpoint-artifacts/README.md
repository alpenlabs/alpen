# Checkpoint artifact registry

The integrated checkpoint prover keeps resident programs indexed by `OLSpecId`.
For each completed epoch it reads the current spec from that epoch's terminal OL
state and looks up its program. This records the spec the epoch actually used,
including V0 epochs and the first V1 epoch after the upgrade. The terminal state's
staged spec applies to the next epoch. The same selection drives DA computation
and routing to the spec's fixed-host prover service.

The registry contains no activation heights, epoch ranges, or mutable active
program. ASM's current active predicate is not a selector for every local proof:
the OL may be ahead of checkpoint acceptance, or proving older work.

## Configuration

Native proving uses the supported built-in V1 program. SP1 deployments can
configure independent release bundles:

```toml
[prover]
backend = "sp1"

[[prover.artifacts]]
spec = 1
bundle_dir = "elfs/sp1/v1"
```

Each V1 bundle contains `guest-checkpoint.elf`, `guest-checkpoint.predicate`, and
`guest-checkpoint.artifact-manifest.json`. The schema 1 manifest records `spec`,
`program_id`, and `runtime_params_hash`. Loading derives the program identity and
predicate from the ELF and checks both sidecars and the node's runtime parameters
before exposing the host. Unknown specs and duplicate spec declarations are
configuration errors. V0 proving is not supported. Startup fails if any active or
pending checkpoint VK has no matching loaded artifact.

### V0 artifacts during the upgrade

If local ASM state still has the deployed V0 key, configure its artifact alongside
V1 so the startup check can validate both keys:

```toml
[[prover.artifacts]]
spec = 0
bundle_dir = "elfs/sp1/v0"

[[prover.artifacts]]
spec = 1
bundle_dir = "elfs/sp1/v1"
```

The V0 directory contains the deployed `guest-checkpoint.elf` and a
`guest-checkpoint.predicate` sidecar holding its published predicate string. Copy
the ELF from the deployed release image and the predicate from that deployment's
configuration. Do not rebuild the old guest or derive the expected predicate with
the new binary. Loading derives the complete predicate from the ELF and requires
an exact match; the usual startup check then compares loaded predicates with ASM.

V0 loading uses only the ELF and predicate; it does not read a manifest.
V1 still requires its full manifest, including the matching runtime-parameter hash.

V0 is loaded only for startup validation and never gets a proving service or task
recovery. The 0.3.x sequencer proves the final V0 epoch; the promoted node starts at
the following V1 epoch. Once local ASM state requires only V1, the V0 entry can be
removed. Promotion and V0 checkpoint replay are handled by STR-4486.

SP1 proving requires at least one explicit artifact entry, even for a single
program. Each entry must name its spec and bundle directory. V1 bundles must also
have a manifest declaring the spec. Missing configuration fails startup; there is no default
bundle directory or inferred V1 program. To use locally built artifacts, configure
`bundle_dir` to point to the builder's output directory. Existing deployments must
add artifact entries and supply V1 manifests with the spec field. Artifact publishing
includes the per-guest manifest and its checksum.

## Availability and recovery

Every configured bundle is loaded and its host initialized once at startup.
Any loading or validation failure stops startup with the spec, bundle directory,
and underlying cause, even if ASM does not yet require that artifact. The registry
contains only successfully validated artifacts. There is no hot reload or automatic
ELF download; supply corrected or additional bundles and restart. An unsupported
spec requires a binary upgrade that implements its rules; an ELF alone cannot
add support for an unknown spec.

Before launching prover services or recovering tasks, startup requires a matching
resident artifact for every active and enacted-pending predicate in the locally
canonical ASM checkpoint state. This check does not wait for L1 or ASM to catch up
to Bitcoin's tip. The initial persisted genesis anchor is usable before the L1
canonical index exists. Missing required artifacts fail startup.
Extra validated artifacts are allowed even when checkpoint state does not yet
reference their predicates; later state updates can use those resident artifacts.

When starting a sequencer with `--bootstrap-from-checkpoint`, the node saves its
promotion before the ASM-based prover configuration check. If that check rejects
startup, correct the prover configuration and rerun the same command. Promotion
is idempotent, so this retry is safe.

The startup check compares VKs only. It does not read OL state or infer a spec for
any predicate. Each epoch selects its artifact from its terminal OL state's current spec.
The OL may already use a VK that is still pending in ASM.

The prover does not repeat the ASM check during operation. Existing ASM rotation
logs can prompt operators to check deployment readiness, but do not report whether
the local prover has the required ELF. Cancellable administration proposals are
outside these checks.

If an epoch requires a missing or unsupported artifact, the OL runner reports an
operator-configuration error and waits at that epoch without submitting a proof or
consuming retry budgets. Input resolution checks the task's canonical commitment
and required spec before assembling a witness, including recovery and retry
attempts. An artifact first needed after startup follows this policy, including
after L1 catch-up.

Each supported proving spec has its own fixed-host prover service. V0 has none.
The spec-scoped task
store filters both unfinished-task recovery and due retries; a service never
claims another spec's work. Like EE, `VersionedTaskStore` prepends the spec to
the unchanged task key and strips it before returning records to the prover.
The prefix is four bytes because OL spec identifiers are u32. Old unprefixed
tasks are ignored; they are not migrated or resumed. Offline dbtool backfill
writes V1-prefixed keys; backfill for other specs and decoded task details are deferred.

Remote jobs store an opaque request ID in task metadata. The remote strategy can
resume that request through the same spec's service while its metadata remains available.

### Checkpoint work across restarts

Startup preserves local checkpoint payloads, signing records, proofs, and prover
tasks, including work beyond ASM's verified tip. An epoch can be proven before
its checkpoint is published or accepted on L1. A lagging local ASM tip therefore
does not invalidate that work. Completed proofs are reused, and unfinished tasks
retain their remote request IDs and retry state for the existing recovery path.

Protocol upgrades assign rules to epochs through their committed spec; they do
not retroactively change the rules for completed epochs. Restart with the same
protocol artifact for each spec whose work must resume. A missing or invalid
artifact is a configuration problem, handled by the startup checks and per-epoch
routing described above, rather than by deleting stored work. Replacing an
artifact under an existing spec or changing a network's runtime parameters is
not a protocol upgrade or a supported way to migrate persisted proof tasks.

The sequencer only incorporates ASM manifests after the configured L1 burial
depth. Supported shallow L1 reorgs can remove a checkpoint's publication without
invalidating the OL execution it proves. Reorgs crossing that safety boundary
require recovery beyond the supported shallow-reorg path; startup does not try
to repair them by clearing checkpoints. A task whose epoch commitment no longer
matches the canonical summary is rejected by input resolution before proving.

There is no automatic checkpoint deletion on restart, including when artifact
validation fails. Offline repair remains an explicit operator action. If repair
removes a proof that must be regenerated, its completed task and any dependent
local payload/signing state must also be reconciled; deleting only the receipt
does not reset the prover's completed status. L1 writer-intent cancellation is a
separate concern tracked by STR-4290.

The remote strategy fetches the receipt and decodes its public output without
adding local cryptographic proof verification. Each service
checks the task's terminal-state spec before proving. Missing terminal state
is retried without a version fallback. The witness still starts from the previous
epoch's terminal state; if that state is missing, witness resolution waits.
Node routing checks artifact availability before submitting work. The
shared prover-core needs no changes for this routing. ASM checks run only at startup.

## Checks

Focused tests cover bundle identity, per-epoch selection, missing-artifact errors,
startup artifact checks against ASM state, and restart recovery. A real SP1 bundle integration test
is opt-in because it requires a built checkpoint ELF and its runtime parameters:

```sh
SP1_PROVER=cpu \
CHECKPOINT_ARTIFACT_TEST_BUNDLE="$PWD/elfs/sp1/v1" \
CHECKPOINT_RUNTIME_PARAMS_PATH="$PWD/.github/fixtures/checkpoint-runtime-params.json" \
cargo test --release -p strata-ol-checkpoint-artifacts --features sp1 \
  loads_real_bundle_and_rejects_tampered_identity -- --ignored --nocapture
```

Use runtime parameters matching the bundle's build. Live rotation after V1 also
requires the separately owned OL spec-activation implementation and an actual
supported successor program.

To check the deployed V0 ELF against its independently published predicate:

```sh
SP1_PROVER=cpu \
CHECKPOINT_V0_ARTIFACT_TEST_BUNDLE="$PWD/elfs/sp1/v0" \
cargo test --release -p strata-ol-checkpoint-artifacts --features sp1 \
  validates_deployed_v0_without_enabling_proving -- --ignored --nocapture
```

This checks that the current SP1 SDK derives the expected V0 predicate and that
the loaded V0 artifact cannot create a proving service. It does not generate a proof.
