"""
Tests that a late-joining fullnode syncs historical blocks from another fullnode.

Scenario:
1. Start sequencer + fullnode_0, produce blocks
2. Start fullnode_1 connecting only to fullnode_0
3. fullnode_1 should sync historical blocks from fullnode_0
"""

import contextlib
import logging
from pathlib import Path

import flexitest

from common.base_test import AlpenClientTest
from common.config.constants import ServiceType
from common.wait import wait_until
from factories.alpen_client import AlpenClientFactory, generate_sequencer_keypair

logger = logging.getLogger(__name__)


def wait_for_canonical_block_log(ee_node, block_number: int, block_hash: str, timeout: int) -> None:
    """Wait until Reth reports the block as added to its canonical chain."""
    log_path = Path(ee_node.props["datadir"]) / "service.log"
    needle = f"Block added to canonical chain number={block_number} hash={block_hash}"

    def has_canonical_block_log() -> bool:
        if not log_path.exists():
            return False

        with log_path.open(encoding="utf-8", errors="ignore") as log_file:
            return any(needle in line for line in log_file)

    wait_until(
        has_canonical_block_log,
        error_with=f"Canonical block {block_number} with hash {block_hash} not observed",
        timeout=timeout,
    )


@flexitest.register
class TestFullnodeSync(AlpenClientTest):
    """Test historical block sync from fullnode to late-joining fullnode."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        ee_sequencer = self.get_service(ServiceType.AlpenSequencer)
        ee_fullnode_0 = self.get_service(ServiceType.AlpenFullNode)

        _, pubkey = generate_sequencer_keypair()
        factory = AlpenClientFactory(range(30600, 30700))

        # Wait for initial sync
        logger.info("Waiting for initial sync...")
        ee_sequencer.wait_for_peers(1, timeout=60)
        ee_fullnode_0.wait_for_peers(1, timeout=60)

        # Produce blocks
        initial_block = ee_sequencer.get_block_number()
        target_block = initial_block + 10
        ee_sequencer.wait_for_additional_blocks(10)
        ee_fullnode_0.wait_for_block(target_block)

        expected_hash = ee_fullnode_0.get_block_by_number(target_block)["hash"]
        fn0_enode = ee_fullnode_0.get_enode()

        # Start late-joining ee_fullnode_1
        logger.info("Starting late-joining ee_fullnode_1...")
        tmpdir = Path(ee_fullnode_0.props["datadir"]).parent / "ee_fullnode_1"
        ee_fullnode_1 = None
        try:
            ee_fullnode_1 = factory.create_fullnode(
                sequencer_pubkey=pubkey,
                trusted_peers=[fn0_enode],
                bootnodes=None,
                enable_discovery=False,
                instance_id=1,
                datadir_override=str(tmpdir),
            )
            ee_fullnode_1.wait_for_ready(timeout=30)

            # Connect ee_fullnode_1 to ee_fullnode_0
            fn0_rpc = ee_fullnode_0.create_rpc()
            fn0_rpc.admin_addPeer(ee_fullnode_1.get_enode())

            ee_fullnode_1.wait_for_peers(1, timeout=60)

            # Verify historical sync
            historical_sync_timeout = ee_fullnode_1.get_block_wait_timeout(
                target_block,
                timeout_per_block=40.0,
                timeout_slack=60,
            )
            wait_for_canonical_block_log(
                ee_fullnode_1,
                target_block,
                expected_hash,
                timeout=historical_sync_timeout,
            )

            # Verify new block relay
            new_target = target_block + 5
            ee_sequencer.wait_for_additional_blocks(5)
            expected_new_hash = ee_sequencer.get_block_by_number(new_target)["hash"]
            new_block_sync_timeout = ee_fullnode_1.get_block_wait_timeout(
                new_target - target_block,
                timeout_per_block=40.0,
                timeout_slack=60,
            )
            wait_for_canonical_block_log(
                ee_fullnode_1,
                new_target,
                expected_new_hash,
                timeout=new_block_sync_timeout,
            )

            logger.info(f"ee_fullnode_1 synced block {target_block} and new block {new_target}")
            return True
        finally:
            if ee_fullnode_1 is not None:
                with contextlib.suppress(Exception):
                    ee_fullnode_1.stop()
