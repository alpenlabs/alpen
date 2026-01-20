"""
Alpen-client multi-fullnode block propagation test.

Tests that multiple fullnodes can connect to sequencer and receive blocks.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)

FULLNODE_COUNT = 3


@flexitest.register
class TestMultiFullnodeBlockPropagation(AlpenClientTest):
    """
    Test block propagation from sequencer to multiple fullnodes.

    Environment: 1 sequencer + 3 fullnodes (star topology)
    All fullnodes connect to sequencer via admin_addPeer.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client_multi")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnodes = [self.get_service(f"fullnode_{i}") for i in range(FULLNODE_COUNT)]

        logger.info("Waiting for all nodes to be ready...")
        sequencer.wait_for_ready(timeout=60)
        for i, fn in enumerate(fullnodes):
            fn.wait_for_ready(timeout=60)
            logger.info(f"Fullnode {i} is ready")

        # Wait for peer connections
        logger.info("Waiting for P2P connections...")
        sequencer.wait_for_peers(FULLNODE_COUNT, timeout=60)
        for i, fn in enumerate(fullnodes):
            fn.wait_for_peers(1, timeout=30)
            logger.info(f"Fullnode {i} connected to sequencer")

        # Verify sequencer sees all fullnodes
        seq_peer_count = sequencer.get_peer_count()
        logger.info(f"Sequencer has {seq_peer_count} peers")
        assert seq_peer_count >= FULLNODE_COUNT, (
            f"Sequencer should have at least {FULLNODE_COUNT} peers, got {seq_peer_count}"
        )

        # Get initial block number from sequencer
        seq_block = sequencer.get_block_number()
        logger.info(f"Sequencer at block {seq_block}")

        # Wait for sequencer to produce a few more blocks
        target_block = seq_block + 5
        logger.info(f"Waiting for sequencer to reach block {target_block}...")
        sequencer.wait_for_block(target_block, timeout=60)

        # Get block info from sequencer
        seq_block_info = sequencer.get_block_by_number(target_block)
        assert seq_block_info is not None, f"Sequencer missing block {target_block}"
        seq_block_hash = seq_block_info["hash"]
        logger.info(f"Sequencer block {target_block} hash: {seq_block_hash}")

        # Wait for ALL fullnodes to receive the block and verify hashes match
        for i, fn in enumerate(fullnodes):
            logger.info(f"Waiting for fullnode {i} to receive block {target_block}...")
            fn.wait_for_block(target_block, timeout=60)

            fn_block_info = fn.get_block_by_number(target_block)
            assert fn_block_info is not None, f"Fullnode {i} missing block {target_block}"
            fn_block_hash = fn_block_info["hash"]
            logger.info(f"Fullnode {i} block {target_block} hash: {fn_block_hash}")

            assert seq_block_hash == fn_block_hash, (
                f"Block hash mismatch for fullnode {i}! "
                f"Sequencer: {seq_block_hash}, Fullnode: {fn_block_hash}"
            )

        logger.info(f"All {FULLNODE_COUNT} fullnodes received block {target_block} correctly!")
        return True
