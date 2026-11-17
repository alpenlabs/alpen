"""A fresh V2 network rotates its checkpoint predicate to V3 on every test run.

SP1 mode proves V2 with its frozen published bundle and V3 with the guest built
from the checkout.
"""

import flexitest

from tests.checkpoint.predicate_rotation import CheckpointPredicateRotation


@flexitest.register
class TestCheckpointPredicateRotationV2V3(CheckpointPredicateRotation):
    """Rotates a V2 network to V3."""

    def __init__(self, ctx: flexitest.InitContext):
        super().__init__(ctx, genesis_spec=2)
