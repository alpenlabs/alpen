"""
Alpen-client bootnode discovery test.

Tests peer discovery via bootnode (discv5).
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestBootnodeDiscovery(AlpenClientTest):
    """
    Test peer discovery via bootnode (discv5).

    This test uses the discovery environment where:
    - Sequencer has discovery enabled (acts as bootnode)
    - Fullnode connects via --bootnodes (no --trusted-peers)

    NOTE: This test will FAIL if discovery is disabled in alpen-client.
    The failure indicates we need to fix main.rs:71 (disable_discovery = true).
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client_discovery")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode = self.get_service("fullnode")

        logger.info("Waiting for nodes to be ready...")
        sequencer.wait_for_ready(timeout=60)
        fullnode.wait_for_ready(timeout=60)

        # Log node info for debugging
        seq_info = sequencer.get_node_info()
        fn_info = fullnode.get_node_info()
        logger.info(f"Sequencer enode: {seq_info.get('enode', 'N/A')}")
        logger.info(f"Fullnode enode: {fn_info.get('enode', 'N/A')}")

        # Wait for peer discovery to work
        # This is the key test - without trusted-peers, nodes should discover via bootnode
        logger.info("Waiting for peers to discover each other via discv5...")

        try:
            sequencer.wait_for_peers(1, timeout=60)
            fullnode.wait_for_peers(1, timeout=60)
        except AssertionError as e:
            logger.error("Discovery failed! Nodes did not connect.")
            logger.error("This likely means discovery is disabled in alpen-client.")
            logger.error("Check bin/alpen-client/src/main.rs:71")
            raise AssertionError(
                "Bootnode discovery failed. Discovery may be disabled in alpen-client. "
                "See main.rs:71 - disable_discovery = true"
            ) from e

        # Verify connection
        seq_peers = sequencer.get_peer_count()
        fn_peers = fullnode.get_peer_count()
        logger.info(f"Sequencer peers: {seq_peers}, Fullnode peers: {fn_peers}")

        assert seq_peers >= 1, f"Sequencer has no peers: {seq_peers}"
        assert fn_peers >= 1, f"Fullnode has no peers: {fn_peers}"

        logger.info("Bootnode discovery test passed!")
        return True
