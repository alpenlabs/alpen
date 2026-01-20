"""
Alpen-client fullnode relay test.

Tests that a fullnode can act as a bootnode and relay blocks between
sequencer and other fullnodes.

Topology:
    fullnode_0 (bootnode)
        > sequencer (connects to fullnode_0)
        > fullnode_1 (connects to fullnode_0)

Sequencer has no direct connection to fullnode_1.
Blocks must relay: sequencer → fullnode_0 → fullnode_1
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestFullnodeRelay(AlpenClientTest):
    """
    Test that blocks relay through a fullnode bootnode.

    Topology: sequencer ↔ fullnode_0 ↔ fullnode_1
    fullnode_1 has NO direct connection to sequencer.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client_relay")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode_0 = self.get_service("fullnode_0")
        fullnode_1 = self.get_service("fullnode_1")

        # Wait for peer connections to establish
        logger.info("Waiting for P2P connections...")

        # fullnode_0 should have 2 peers (sequencer + fullnode_1)
        fullnode_0.wait_for_peers(2, timeout=60)
        fn0_peers = fullnode_0.get_peer_count()
        logger.info(f"fullnode_0 (bootnode) has {fn0_peers} peers")

        # sequencer should have 1 peer (fullnode_0)
        sequencer.wait_for_peers(1, timeout=30)
        seq_peers = sequencer.get_peer_count()
        logger.info(f"Sequencer has {seq_peers} peers")

        # fullnode_1 should have 1 peer (fullnode_0)
        fullnode_1.wait_for_peers(1, timeout=30)
        fn1_peers = fullnode_1.get_peer_count()
        logger.info(f"fullnode_1 has {fn1_peers} peers")

        # Verify topology: sequencer should NOT be directly connected to fullnode_1
        seq_peer_list = sequencer.get_peers()
        fn1_enode = fullnode_1.get_enode()
        fn1_id = fn1_enode.split("@")[0].replace("enode://", "")

        seq_peer_ids = [p.get("id", "") for p in seq_peer_list]
        assert fn1_id not in seq_peer_ids, (
            f"Sequencer should NOT be directly connected to fullnode_1! "
            f"Sequencer peers: {seq_peer_ids}, fullnode_1 id: {fn1_id}"
        )
        logger.info("Verified: Sequencer is NOT directly connected to fullnode_1")

        # Get current block number
        seq_block = sequencer.get_block_number()
        logger.info(f"Sequencer at block {seq_block}")

        # Wait for sequencer to produce more blocks
        target_block = seq_block + 5
        logger.info(f"Waiting for sequencer to reach block {target_block}...")
        sequencer.wait_for_block(target_block, timeout=60)

        # Get block info from sequencer
        seq_block_info = sequencer.get_block_by_number(target_block)
        assert seq_block_info is not None, f"Sequencer missing block {target_block}"
        seq_block_hash = seq_block_info["hash"]
        logger.info(f"Sequencer block {target_block} hash: {seq_block_hash}")

        # Verify fullnode_0 received the block (direct from sequencer)
        logger.info(f"Waiting for fullnode_0 to receive block {target_block}...")
        fullnode_0.wait_for_block(target_block, timeout=60)
        fn0_block_info = fullnode_0.get_block_by_number(target_block)
        assert fn0_block_info is not None
        fn0_block_hash = fn0_block_info["hash"]
        logger.info(f"fullnode_0 block {target_block} hash: {fn0_block_hash}")
        assert seq_block_hash == fn0_block_hash, "fullnode_0 hash mismatch!"

        # Verify fullnode_1 received the block (relayed via fullnode_0)
        logger.info(f"Waiting for fullnode_1 to receive block {target_block} (via relay)...")
        fullnode_1.wait_for_block(target_block, timeout=60)
        fn1_block_info = fullnode_1.get_block_by_number(target_block)
        assert fn1_block_info is not None
        fn1_block_hash = fn1_block_info["hash"]
        logger.info(f"fullnode_1 block {target_block} hash: {fn1_block_hash}")
        assert seq_block_hash == fn1_block_hash, "fullnode_1 hash mismatch!"

        logger.info(
            f"SUCCESS: Block {target_block} relayed through fullnode_0!\n"
            f"  sequencer -> fullnode_0 -> fullnode_1\n"
            f"  All hashes match: {seq_block_hash}"
        )
        return True
