"""
Tests that nodes can discover each other purely via discv5 bootnode protocol,
without any admin_addPeer RPC calls.

nodes must find each other organically.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestPureBootnodeDiscovery(AlpenClientTest):
    """
    Test that nodes discover each other purely via discv5 bootnodes.

    Environment setup:
    - Sequencer: discv5 enabled, acts as bootnode
    - Fullnode: discv5 enabled, --bootnodes points to sequencer
    - NO admin_addPeer calls - nodes must discover each other via discv5

    This tests the full discovery flow:
    1. Fullnode queries bootnode (sequencer) via discv5
    2. Nodes exchange ENRs (Ethereum Node Records)
    3. Nodes establish RLPx connection
    4. Blocks propagate via gossip
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client_pure_discovery")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode = self.get_service("fullnode")

        # Log node info for debugging
        seq_info = sequencer.get_node_info()
        fn_info = fullnode.get_node_info()
        logger.info(f"Sequencer enode: {seq_info.get('enode', 'N/A')}")
        logger.info(f"Fullnode enode: {fn_info.get('enode', 'N/A')}")

        # Wait for peer discovery - this is the key test!
        # Without admin_addPeer, nodes must discover each other via discv5
        logger.info("Waiting for nodes to discover each other via discv5 (no admin_addPeer)...")

        try:
            # Give discovery more time since it's organic
            sequencer.wait_for_peers(1, timeout=90)
            fullnode.wait_for_peers(1, timeout=90)
        except AssertionError as e:
            # Log peer info for debugging
            seq_peers = sequencer.get_peer_count()
            fn_peers = fullnode.get_peer_count()
            logger.error(f"Discovery failed! Sequencer peers: {seq_peers}, Fullnode peers: {fn_peers}")
            logger.error("Nodes failed to discover each other via discv5 bootnode protocol.")
            raise AssertionError(
                "Pure bootnode discovery failed. Nodes could not find each other via discv5."
            ) from e

        # Verify connection
        seq_peers = sequencer.get_peer_count()
        fn_peers = fullnode.get_peer_count()
        logger.info(f"Discovery successful! Sequencer peers: {seq_peers}, Fullnode peers: {fn_peers}")

        # Now verify block propagation works
        seq_block = sequencer.get_block_number()
        target_block = seq_block + 3
        logger.info(f"Waiting for sequencer to produce block {target_block}...")
        sequencer.wait_for_block(target_block, timeout=60)

        # Verify fullnode receives blocks
        logger.info(f"Waiting for fullnode to receive block {target_block}...")
        fullnode.wait_for_block(target_block, timeout=60)

        # Verify block hashes match
        seq_block_info = sequencer.get_block_by_number(target_block)
        fn_block_info = fullnode.get_block_by_number(target_block)
        assert seq_block_info is not None and fn_block_info is not None

        seq_hash = seq_block_info["hash"]
        fn_hash = fn_block_info["hash"]
        assert seq_hash == fn_hash, f"Block hash mismatch: {seq_hash} vs {fn_hash}"

        logger.info(
            f"Pure bootnode discovery test passed!\n"
            f"  Nodes discovered each other via discv5 (no admin_addPeer)\n"
            f"  Block {target_block} propagated correctly\n"
            f"  Hash: {seq_hash}"
        )
        return True
