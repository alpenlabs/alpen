# Checkpoint artifact registry

The integrated checkpoint prover keeps resident programs indexed by `OLSpecId`.
For each epoch it reads the committed state at that epoch's previous terminal,
selects the staged spec, and looks up its program. The first V1 epoch starting
from a V0 state follows the checkpoint guest's V0-to-V1 exception. The same
selection drives DA computation and routing to the spec's fixed-host prover service.

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

Each bundle contains `guest-checkpoint.elf`, `guest-checkpoint.predicate`, and
`guest-checkpoint.artifact-manifest.json`. The schema 1 manifest records `spec`,
`program_id`, and `runtime_params_hash`. Loading derives the program identity and
predicate from the ELF and checks both sidecars and the node's runtime parameters
before exposing the host. Unknown specs and duplicate spec declarations are
configuration errors. V0 proving is not supported. Startup fails if any active or
pending checkpoint VK has no matching loaded artifact.

SP1 proving requires at least one explicit artifact entry, even for a single
program. Each entry must name its spec and bundle directory, and each manifest
must declare its spec. Missing configuration fails startup; there is no default
bundle directory or inferred V1 program. To use locally built artifacts, configure
`bundle_dir` to point to the builder's output directory. Existing deployments must
add artifact entries and supply manifests with the spec field. Artifact publishing
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

The startup check compares VKs only. It does not read OL state or infer a spec for
any predicate. Each epoch selects its artifact from its committed OL start state.
The OL may already use a VK that is still pending in ASM.

The prover does not repeat the ASM check during operation. Existing ASM rotation
logs can prompt operators to check deployment readiness, but do not report whether
the local prover has the required ELF. Cancellable administration proposals are
outside these checks.

If an epoch requires a missing or unsupported artifact, the OL runner reports an
operator-configuration error and waits at that epoch without submitting a proof or
consuming retry budgets. Existing records for specs without loaded artifacts remain
untouched until their service can run after restart. Each service checks the task's
canonical commitment and required artifact before assembling a witness, including
recovery and retry attempts. Task admission does not read ASM state. An artifact
first needed after startup follows this policy, including after L1 catch-up.

Each resident spec has its own fixed-host prover service. The spec-scoped task
store filters both unfinished-task recovery and due retries; a service never
claims another spec's work. Like EE, `VersionedTaskStore` prepends the spec to
the unchanged task key and strips it before returning records to the prover.
The prefix is four bytes because OL spec identifiers are u32. Old unprefixed
tasks are ignored; they are not migrated or resumed. Offline dbtool backfill
writes V1-prefixed keys; backfill for other specs and decoded task details are deferred.

Remote jobs retain the existing opaque request-ID metadata. Restart recovery
resumes that same request through the same spec's service, following the EE's
stable routing model. The remote strategy fetches the receipt and decodes its
public output without adding local cryptographic proof verification. Each service
checks the task's committed start-state spec before proving. Missing start state
is retried without a version fallback. Witness resolution handles proof input
readiness; routing and task admission check artifact availability. ASM checks run
only at startup.

## Checks

Focused tests cover bundle identity, per-epoch selection, configuration waits,
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
