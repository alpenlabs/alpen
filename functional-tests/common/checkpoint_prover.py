"""Selects the programs and parameters for the repeatable V1 to V2 upgrade."""

import json
import os
from dataclasses import dataclass
from pathlib import Path

from common.config import ProverConfig
from common.config.params import BridgeParams
from common.datatool import CHECKPOINT_TEST_PREDICATE_FILE, PREDICATE_FIXTURES_DIR

_REPO_ROOT = Path(__file__).resolve().parents[2]
_V1_BUNDLE = _REPO_ROOT / "functional-tests" / "testdata" / "sp1" / "v1"
_V2_BUNDLE = _REPO_ROOT / "provers" / "sp1" / "generated"


@dataclass(frozen=True)
class CheckpointRotationPrograms:
    """Keeps genesis, the rotation target, and runtime parameters coherent."""

    config: ProverConfig
    genesis_predicate_file: Path
    rotation_predicate_file: Path
    bridge_params: BridgeParams
    ol_block_time_ms: int
    proof_wait_scale: int

    @property
    def rotation_predicate(self) -> str:
        return self.rotation_predicate_file.read_text().strip()


def checkpoint_rotation_programs() -> CheckpointRotationPrograms:
    """Selects native execution by default, or the frozen V1 and built V2 SP1 guests."""
    backend = os.environ.get("OL_PROVER_BACKEND", "native")
    if backend == "native":
        return CheckpointRotationPrograms(
            config=ProverConfig(),
            genesis_predicate_file=CHECKPOINT_TEST_PREDICATE_FILE,
            rotation_predicate_file=(
                PREDICATE_FIXTURES_DIR / "checkpoint-v2-bip340-schnorr-test.predicate"
            ),
            bridge_params=BridgeParams(),
            ol_block_time_ms=1_000,
            proof_wait_scale=1,
        )
    if backend != "sp1":
        raise ValueError(f"unknown OL_PROVER_BACKEND {backend!r}; expected native or sp1")

    # A build only produces the current spec. V1 must stay the deployed guest;
    # rebuilding it from current source would stop checking historical compatibility.
    for bundle in (_V1_BUNDLE, _V2_BUNDLE):
        for filename in (
            "guest-checkpoint.elf",
            "guest-checkpoint.predicate",
            "guest-checkpoint.artifact-manifest.json",
        ):
            if not (bundle / filename).is_file():
                recovery = (
                    "restore the committed V1 bundle"
                    if bundle == _V1_BUNDLE
                    else "run OL_PROVER_BACKEND=sp1 SP1_PROVER=network ./run_tests.sh to build V2"
                )
                raise FileNotFoundError(
                    f"missing checkpoint artifact: {bundle / filename}; {recovery}"
                )
    runtime_params = json.loads((_V1_BUNDLE / "checkpoint-runtime-params.json").read_text())
    return CheckpointRotationPrograms(
        config=ProverConfig(
            backend="sp1",
            artifacts=[
                {"spec": 1, "bundle_dir": str(_V1_BUNDLE)},
                {"spec": 2, "bundle_dir": str(_V2_BUNDLE)},
            ],
        ),
        genesis_predicate_file=_V1_BUNDLE / "guest-checkpoint.predicate",
        rotation_predicate_file=_V2_BUNDLE / "guest-checkpoint.predicate",
        bridge_params=BridgeParams(**runtime_params["bridge_params"]),
        # Eight slots now span two minutes, reducing the proof backlog while
        # network requests are pending. Native keeps its fast block cadence.
        ol_block_time_ms=15_000,
        proof_wait_scale=4,
    )
