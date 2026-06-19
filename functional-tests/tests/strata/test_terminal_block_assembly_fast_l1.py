"""Validate terminal block assembly with 2s L1 blocks."""

import flexitest

from tests.strata.helpers import (
    TerminalBlockAssemblyBase,
)


@flexitest.register
class TerminalBlockAssemblyFastL1Test(TerminalBlockAssemblyBase):
    """Validate repeated terminal block assembly with 2s L1 mining."""

    l1_mining_interval_seconds = 2.0
    terminal_blocks_to_validate = 2
    terminal_slot_timeout_seconds = 180
    # Conservative floor: fast L1 should contribute several manifests to each epoch.
    min_l1_manifests_in_epoch = 2
