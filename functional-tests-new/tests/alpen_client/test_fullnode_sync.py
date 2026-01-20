"""
Alpen-client fullnode sync test.

Tests that a late-joining fullnode can sync historical blocks from another fullnode
while the network is actively producing new blocks.

Scenario:
1. Start sequencer + fullnode_0
2. Wait for blocks to be produced and synced to fullnode_0
3. Start fullnode_1 connecting to fullnode_0 only (NOT sequencer)
4. fullnode_1 should sync historical blocks from fullnode_0

This tests that fullnodes can serve as sync sources for other fullnodes.
The key difference from relay test: fullnode_1 syncs HISTORICAL blocks it missed,
not just newly gossiped blocks.
"""

import logging
import tempfile

import flexitest

from common.base_test import AlpenClientTest
from factories.alpen_client import AlpenClientFactory, generate_sequencer_keypair

logger = logging.getLogger(__name__)


@flexitest.register
class TestFullnodeSync(AlpenClientTest):
    """
    Test that a late-joining fullnode can sync historical blocks from another fullnode.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode_0 = self.get_service("fullnode")  # Named fullnode_0 to distinguish from fullnode_1

        # Get pubkey for creating fullnode_1 later
        # Create a new factory with different port range to avoid conflicts
        _, pubkey = generate_sequencer_keypair()
        factory = AlpenClientFactory(range(30600, 30700))

        # Wait for peer connection
        logger.info("Waiting for sequencer and fullnode_0 to connect...")
        sequencer.wait_for_peers(1, timeout=30)
        fullnode_0.wait_for_peers(1, timeout=30)
        logger.info("Sequencer and fullnode_0 connected")

        # Wait for some blocks to be produced
        initial_block = sequencer.get_block_number()
        target_block = initial_block + 10
        logger.info(f"Waiting for sequencer to produce blocks up to {target_block}...")
        sequencer.wait_for_block(target_block, timeout=120)

        # Verify fullnode_0 has synced
        fullnode_0.wait_for_block(target_block, timeout=60)
        fn0_block = fullnode_0.get_block_number()
        logger.info(f"fullnode_0 synced to block {fn0_block}")

        # Get block hash for verification later
        fn0_block_info = fullnode_0.get_block_by_number(target_block)
        assert fn0_block_info is not None
        expected_hash = fn0_block_info["hash"]
        logger.info(f"Block {target_block} hash: {expected_hash}")

        # Get fullnode_0's enode - fullnode_1 will connect ONLY to fullnode_0
        fn0_enode = fullnode_0.get_enode()
        logger.info(f"fullnode_0 enode: {fn0_enode}")

        # Start fullnode_1 connecting ONLY to fullnode_0
        # fullnode_1 must sync historical blocks from fullnode_0 (not sequencer)
        logger.info("Starting fullnode_1 (connecting to fullnode_0 only)...")
        tmpdir = tempfile.mkdtemp(prefix="alpen_fullnode_1_")
        fullnode_1 = None
        try:
            fullnode_1 = factory.create_fullnode(
                sequencer_pubkey=pubkey,
                trusted_peers=[fn0_enode],
                bootnodes=None,
                enable_discovery=False,
                instance_id=1,
                datadir_override=tmpdir,
            )
            fullnode_1.wait_for_ready(timeout=60)

            # Get fullnode_1's enode and explicitly add it to fullnode_0
            fn1_enode = fullnode_1.get_enode()
            logger.info(f"fullnode_1 enode: {fn1_enode}")

            # Use admin_addPeer to establish connection from fullnode_0 to fullnode_1
            fn0_rpc = fullnode_0.create_rpc()
            fn0_rpc.admin_addPeer(fn1_enode)
            logger.info("Added fullnode_1 as peer to fullnode_0 via admin_addPeer")

            # Wait for fullnode_1 to connect to fullnode_0
            logger.info("Waiting for fullnode_1 to connect to fullnode_0...")
            fullnode_1.wait_for_peers(1, timeout=30)
            fn1_peers = fullnode_1.get_peer_count()
            logger.info(f"fullnode_1 has {fn1_peers} peers")

            # fullnode_0 should now have 2 peers (sequencer + fullnode_1)
            fn0_peers = fullnode_0.get_peer_count()
            logger.info(f"fullnode_0 has {fn0_peers} peers")

            # Wait for fullnode_1 to sync blocks from fullnode_0
            # It needs to catch up to the target block (historical blocks it missed)
            logger.info(f"Waiting for fullnode_1 to sync block {target_block} from fullnode_0...")
            fullnode_1.wait_for_block(target_block, timeout=120)

            # Verify block hash matches
            fn1_block_info = fullnode_1.get_block_by_number(target_block)
            assert fn1_block_info is not None, f"fullnode_1 missing block {target_block}"
            fn1_hash = fn1_block_info["hash"]
            logger.info(f"fullnode_1 block {target_block} hash: {fn1_hash}")

            assert expected_hash == fn1_hash, (
                f"Block hash mismatch! Expected: {expected_hash}, Got: {fn1_hash}"
            )

            # Verify fullnode_1 continues to receive new blocks via relay
            # (gossip goes: sequencer -> fullnode_0 -> fullnode_1)
            new_target = target_block + 5
            logger.info(f"Waiting for fullnode_1 to receive new block {new_target}...")
            fullnode_1.wait_for_block(new_target, timeout=60)
            logger.info(f"fullnode_1 synced to block {new_target}")

            logger.info(
                f"SUCCESS: fullnode_1 synced historical block {target_block} from fullnode_0!\n"
                f"  Also received new blocks up to {new_target} via relay.\n"
                f"  Block {target_block} hash: {expected_hash}"
            )
            return True
        finally:
            # Stop fullnode_1 manually since it wasn't registered with the env
            if fullnode_1 is not None:
                try:
                    fullnode_1.stop()
                except Exception as e:
                    logger.warning(f"Error stopping fullnode_1: {e}")
