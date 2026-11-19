"""A fresh V1 network rotates its checkpoint predicate to V2 on every test run.

SP1 mode proves both specs with their frozen published bundles.
"""

import flexitest

from tests.checkpoint.predicate_rotation import CheckpointPredicateRotation


@flexitest.register
class TestCheckpointPredicateRotationV1V2(CheckpointPredicateRotation):
    """Rotates a V1 network to V2."""

    def __init__(self, ctx: flexitest.InitContext):
        super().__init__(ctx, genesis_spec=1)
