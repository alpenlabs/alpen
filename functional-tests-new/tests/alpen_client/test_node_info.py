"""
Alpen-client node info test.

Tests that nodes expose correct info via admin_nodeInfo.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestNodeInfo(AlpenClientTest):
    """Test that nodes expose correct info via admin_nodeInfo."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode = self.get_service("fullnode")

        logger.info("Waiting for nodes to be ready...")
        sequencer.wait_for_ready(timeout=60)
        fullnode.wait_for_ready(timeout=60)

        # Get node info
        seq_info = sequencer.get_node_info()
        fn_info = fullnode.get_node_info()

        logger.info(f"Sequencer enode: {seq_info.get('enode', 'N/A')}")
        logger.info(f"Fullnode enode: {fn_info.get('enode', 'N/A')}")

        # Verify enode URLs are valid
        seq_enode = seq_info.get("enode", "")
        fn_enode = fn_info.get("enode", "")

        assert seq_enode.startswith("enode://"), f"Invalid sequencer enode: {seq_enode}"
        assert fn_enode.startswith("enode://"), f"Invalid fullnode enode: {fn_enode}"

        # Verify they have different enodes (different nodes)
        assert seq_enode != fn_enode, "Sequencer and fullnode have same enode!"

        logger.info("Node info test passed!")
        return True
