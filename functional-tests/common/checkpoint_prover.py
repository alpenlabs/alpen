"""Selects the programs and parameters for the repeatable checkpoint predicate rotations."""

import json
import os
from dataclasses import dataclass
from pathlib import Path

from common.config import ProverConfig
from common.config.params import BridgeParams
from common.datatool import CHECKPOINT_TEST_PREDICATE_FILE, PREDICATE_FIXTURES_DIR

_REPO_ROOT = Path(__file__).resolve().parents[2]

# Native checkpoint programs sign with a distinct test key per spec.
_NATIVE_PREDICATE_FILES = {
    1: CHECKPOINT_TEST_PREDICATE_FILE,
    2: PREDICATE_FIXTURES_DIR / "checkpoint-v2-bip340-schnorr-test.predicate",
    3: PREDICATE_FIXTURES_DIR / "checkpoint-v3-bip340-schnorr-test.predicate",
}

# Published SP1 programs for specs that networks already run. A build only
# produces the current spec, and rebuilding these from current source would
# stop checking historical compatibility.
_FROZEN_SP1_BUNDLES = {
    1: _REPO_ROOT / "functional-tests" / "testdata" / "sp1" / "v1",
    2: _REPO_ROOT / "functional-tests" / "testdata" / "sp1" / "v2",
}

# The spec the checkout's guest proves, and where run_tests.sh builds it.
_BUILT_SP1_SPEC = 3
_BUILT_SP1_BUNDLE = _REPO_ROOT / "provers" / "sp1" / "generated"

_SP1_BUNDLE_FILES = (
    "guest-checkpoint.elf",
    "guest-checkpoint.predicate",
    "guest-checkpoint.artifact-manifest.json",
)


@dataclass(frozen=True)
class CheckpointRotationPrograms:
    """Keeps the genesis spec, the rotation target, and runtime parameters coherent."""

    genesis_spec: int
    rotation_spec: int
    config: ProverConfig
    genesis_predicate_file: Path
    rotation_predicate_file: Path
    bridge_params: BridgeParams
    ol_block_time_ms: int
    proof_wait_scale: int

    @property
    def rotation_predicate(self) -> str:
        return self.rotation_predicate_file.read_text().strip()


def checkpoint_rotation_programs(genesis_spec: int) -> CheckpointRotationPrograms:
    """Selects the programs for a network that starts at `genesis_spec` and rotates to the next.

    Native execution is the default. SP1 mode uses the frozen bundle of each
    published spec and the guest built from the checkout for the spec it proves.
    """
    rotation_spec = genesis_spec + 1
    backend = os.environ.get("OL_PROVER_BACKEND", "native")
    if backend == "native":
        for spec in (genesis_spec, rotation_spec):
            if spec not in _NATIVE_PREDICATE_FILES:
                raise ValueError(f"no native checkpoint predicate fixture for V{spec}")
        return CheckpointRotationPrograms(
            genesis_spec=genesis_spec,
            rotation_spec=rotation_spec,
            config=ProverConfig(),
            genesis_predicate_file=_NATIVE_PREDICATE_FILES[genesis_spec],
            rotation_predicate_file=_NATIVE_PREDICATE_FILES[rotation_spec],
            bridge_params=BridgeParams(),
            ol_block_time_ms=1_000,
            proof_wait_scale=1,
        )
    if backend != "sp1":
        raise ValueError(f"unknown OL_PROVER_BACKEND {backend!r}; expected native or sp1")

    # The runtime parameters come from the genesis bundle, and only a frozen
    # bundle carries them. Every frozen bundle and the guest build use the same
    # ones; the node rejects any bundle whose manifest disagrees with them.
    if genesis_spec not in _FROZEN_SP1_BUNDLES:
        raise ValueError(f"SP1 mode has no frozen bundle to start a network at V{genesis_spec}")
    genesis_bundle = _sp1_bundle(genesis_spec)
    rotation_bundle = _sp1_bundle(rotation_spec)
    runtime_params = json.loads((genesis_bundle / "checkpoint-runtime-params.json").read_text())
    return CheckpointRotationPrograms(
        genesis_spec=genesis_spec,
        rotation_spec=rotation_spec,
        config=ProverConfig(
            backend="sp1",
            artifacts=[
                {"spec": genesis_spec, "bundle_dir": str(genesis_bundle)},
                {"spec": rotation_spec, "bundle_dir": str(rotation_bundle)},
            ],
        ),
        genesis_predicate_file=genesis_bundle / "guest-checkpoint.predicate",
        rotation_predicate_file=rotation_bundle / "guest-checkpoint.predicate",
        bridge_params=BridgeParams(**runtime_params["bridge_params"]),
        # Eight slots now span two minutes, reducing the proof backlog while
        # network requests are pending. Native keeps its fast block cadence.
        ol_block_time_ms=15_000,
        proof_wait_scale=4,
    )


def _sp1_bundle(spec: int) -> Path:
    """Returns the SP1 bundle directory for `spec`, after checking its files exist."""
    if spec in _FROZEN_SP1_BUNDLES:
        bundle = _FROZEN_SP1_BUNDLES[spec]
        recovery = f"restore the committed V{spec} bundle"
    elif spec == _BUILT_SP1_SPEC:
        bundle = _BUILT_SP1_BUNDLE
        recovery = f"run OL_PROVER_BACKEND=sp1 SP1_PROVER=network ./run_tests.sh to build V{spec}"
    else:
        raise ValueError(f"no SP1 checkpoint program is available for V{spec}")

    for filename in _SP1_BUNDLE_FILES:
        if not (bundle / filename).is_file():
            raise FileNotFoundError(f"missing checkpoint artifact: {bundle / filename}; {recovery}")
    return bundle
