"""Validate terminal block assembly with 20s L1 blocks."""

import flexitest

from tests.strata.helpers import TerminalBlockAssemblyBase


@flexitest.register
class TerminalBlockAssemblySlowL1Test(TerminalBlockAssemblyBase):
    """Validate terminal block assembly with 20s L1 blocks."""

    l1_mining_interval_seconds = 20.0
    terminal_blocks_to_validate = 1
    terminal_slot_timeout_seconds = 120
    # Baseline floor: slow L1 should still contribute a manifest to the terminal block.
    min_l1_manifests_in_terminal_block = 1
