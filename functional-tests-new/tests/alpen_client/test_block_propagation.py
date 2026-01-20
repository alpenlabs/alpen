"""
Alpen-client block propagation test.

Tests blocks propagate from sequencer to fullnode via gossip.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestBlockPropagation(AlpenClientTest):
    """Test blocks propagate from sequencer to fullnode via gossip."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode = self.get_service("fullnode")

        logger.info("Waiting for nodes to be ready...")
        sequencer.wait_for_ready(timeout=60)
        fullnode.wait_for_ready(timeout=60)

        # Wait for peer connection first
        logger.info("Waiting for P2P connection...")
        sequencer.wait_for_peers(1, timeout=30)
        fullnode.wait_for_peers(1, timeout=30)

        # Get initial block number from sequencer
        seq_block = sequencer.get_block_number()
        logger.info(f"Sequencer at block {seq_block}")

        # Wait for sequencer to produce a few blocks
        target_block = seq_block + 6
        logger.info(f"Waiting for sequencer to reach block {target_block}...")
        sequencer.wait_for_block(target_block, timeout=60)

        # Get block info from sequencer
        seq_block_info = sequencer.get_block_by_number(target_block)
        assert seq_block_info is not None, f"Sequencer missing block {target_block}"
        seq_block_hash = seq_block_info["hash"]
        logger.info(f"Sequencer block {target_block} hash: {seq_block_hash}")

        # Wait for fullnode to receive the block
        logger.info(f"Waiting for fullnode to receive block {target_block}...")
        fullnode.wait_for_block(target_block, timeout=60)

        # Verify block hash matches
        fn_block_info = fullnode.get_block_by_number(target_block)
        assert fn_block_info is not None, f"Fullnode missing block {target_block}"
        fn_block_hash = fn_block_info["hash"]
        logger.info(f"Fullnode block {target_block} hash: {fn_block_hash}")

        assert seq_block_hash == fn_block_hash, (
            f"Block hash mismatch! Sequencer: {seq_block_hash}, Fullnode: {fn_block_hash}"
        )

        logger.info("Block propagation test passed!")
        return True
