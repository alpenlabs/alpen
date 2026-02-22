"""
Tests that a late-joining fullnode syncs historical blocks from another fullnode.

Scenario:
1. Start sequencer + fullnode_0, produce blocks
2. Start fullnode_1 connecting only to fullnode_0
3. fullnode_1 should sync historical blocks from fullnode_0
"""

import contextlib
import logging
import tempfile

import flexitest

from common.base_test import AlpenClientTest
from factories.alpen_client import AlpenClientFactory, generate_sequencer_keypair

logger = logging.getLogger(__name__)


@flexitest.register
class TestFullnodeSync(AlpenClientTest):
    """Test historical block sync from fullnode to late-joining fullnode."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("alpen_sequencer")
        fullnode_0 = self.get_service("alpen_fullnode")

        _, pubkey = generate_sequencer_keypair()
        factory = AlpenClientFactory(range(30600, 30700))

        # Wait for initial sync
        logger.info("Waiting for initial sync...")
        sequencer.wait_for_peers(1, timeout=30)
        fullnode_0.wait_for_peers(1, timeout=30)

        # Produce blocks
        initial_block = sequencer.get_block_number()
        target_block = initial_block + 10
        sequencer.wait_for_block(target_block, timeout=120)
        fullnode_0.wait_for_block(target_block, timeout=60)

        expected_hash = fullnode_0.get_block_by_number(target_block)["hash"]
        fn0_enode = fullnode_0.get_enode()

        # Start late-joining fullnode_1
        logger.info("Starting late-joining fullnode_1...")
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

            # Connect fullnode_1 to fullnode_0
            fn0_rpc = fullnode_0.create_rpc()
            fn0_rpc.admin_addPeer(fullnode_1.get_enode())

            fullnode_1.wait_for_peers(1, timeout=30)

            # Verify historical sync
            fullnode_1.wait_for_block(target_block, timeout=120)
            fn1_hash = fullnode_1.get_block_by_number(target_block)["hash"]
            assert expected_hash == fn1_hash, "Historical block hash mismatch"

            # Verify new block relay
            new_target = target_block + 5
            sequencer.wait_for_block(new_target, timeout=120)
            fullnode_1.wait_for_block(new_target, timeout=60)

            logger.info(f"fullnode_1 synced block {target_block} and new block {new_target}")
            return True
        finally:
            if fullnode_1 is not None:
                with contextlib.suppress(Exception):
                    fullnode_1.stop()
