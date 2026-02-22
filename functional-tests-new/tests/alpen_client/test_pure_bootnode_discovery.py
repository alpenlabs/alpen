"""
Tests pure discv5 bootnode discovery without RPC-based peer connection.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestPureBootnodeDiscovery(AlpenClientTest):
    """Test that nodes discover each other purely via discv5 bootnodes."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client_discovery")

    def main(self, ctx):
        sequencer = self.get_service("alpen_sequencer")
        fullnode = self.get_service("alpen_fullnode")

        logger.info("Waiting for discv5 peer discovery...")
        sequencer.wait_for_peers(1, timeout=60)
        fullnode.wait_for_peers(1, timeout=60)
        logger.info("Peers discovered")

        # Verify block propagation
        seq_block = sequencer.get_block_number()
        target_block = seq_block + 3

        sequencer.wait_for_block(target_block, timeout=60)
        fullnode.wait_for_block(target_block, timeout=60)

        seq_hash = sequencer.get_block_by_number(target_block)["hash"]
        fn_hash = fullnode.get_block_by_number(target_block)["hash"]
        assert seq_hash == fn_hash, f"Block hash mismatch: {seq_hash} vs {fn_hash}"

        logger.info(f"Block {target_block} propagated via discv5 mesh")
        return True
