"""
Alpen-client trusted-peers connectivity test.

Tests sequencer-fullnode connectivity via trusted-peers.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestTrustedPeersConnect(AlpenClientTest):
    """Test sequencer and fullnode connect via --trusted-peers."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode = self.get_service("fullnode")

        logger.info("Waiting for nodes to be ready...")
        sequencer.wait_for_ready(timeout=60)
        fullnode.wait_for_ready(timeout=60)

        # Check sequencer has peers
        logger.info("Checking sequencer peer count...")
        seq_peers = sequencer.get_peer_count()
        logger.info(f"Sequencer has {seq_peers} peers")

        # Check fullnode has peers
        logger.info("Checking fullnode peer count...")
        fn_peers = fullnode.get_peer_count()
        logger.info(f"Fullnode has {fn_peers} peers")

        # Wait for peer connection
        logger.info("Waiting for peers to connect...")
        sequencer.wait_for_peers(1, timeout=30)
        fullnode.wait_for_peers(1, timeout=30)

        logger.info("Peers connected successfully!")
        return True
