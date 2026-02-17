"""
Tests block propagation from sequencer to multiple fullnodes.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)

FULLNODE_COUNT = 3


@flexitest.register
class TestMultiFullnodeBlockPropagation(AlpenClientTest):
    """Test block propagation to multiple fullnodes (star topology)."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee_multi")

    def main(self, ctx):
        ee_sequencer = self.get_service("ee_sequencer")
        ee_fullnodes = [self.get_service(f"ee_fullnode_{i}") for i in range(FULLNODE_COUNT)]

        # Wait for connections
        logger.info("Waiting for P2P connections...")
        ee_sequencer.wait_for_peers(FULLNODE_COUNT, timeout=60)
        for fn in ee_fullnodes:
            fn.wait_for_peers(1, timeout=30)

        # Verify block propagation
        seq_block = ee_sequencer.get_block_number()
        target_block = seq_block + 5

        ee_sequencer.wait_for_block(target_block, timeout=60)
        seq_hash = ee_sequencer.get_block_by_number(target_block)["hash"]

        for i, fn in enumerate(ee_fullnodes):
            fn.wait_for_block(target_block, timeout=60)
            fn_hash = fn.get_block_by_number(target_block)["hash"]
            assert seq_hash == fn_hash, f"Fullnode {i} hash mismatch"

        logger.info(f"Block {target_block} propagated to {FULLNODE_COUNT} fullnodes")
        return True
