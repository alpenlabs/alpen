"""Test chain status and protocol version RPC methods."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestChainStatus(StrataNodeTest):
    """Test that chain status reports valid structure and progresses over time."""

    BLOCKS_TO_WAIT = 3

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        rpc = strata.wait_for_rpc_ready(timeout=10)

        # Verify protocol version
        version = rpc.strata_protocolVersion()
        assert version == 1, f"Expected protocol version 1, got {version}"
        logger.info("Protocol version: %d", version)

        # Verify chain status structure
        status = rpc.strata_getChainStatus()
        logger.info("Initial chain status: %s", status)

        assert "latest" in status, "Chain status missing 'latest'"
        assert "slot" in status["latest"], "latest missing 'slot'"
        assert "blkid" in status["latest"], "latest missing 'blkid'"
        assert "parent" in status, "Chain status missing 'parent'"
        assert "epoch" in status["parent"], "parent missing 'epoch'"
        assert "confirmed" in status, "Chain status missing 'confirmed'"
        assert "finalized" in status, "Chain status missing 'finalized'"

        initial_slot = status["latest"]["slot"]

        # Wait for blocks and verify chain progresses
        strata.wait_for_additional_blocks(self.BLOCKS_TO_WAIT, rpc)

        updated_status = rpc.strata_getChainStatus()
        updated_slot = updated_status["latest"]["slot"]
        logger.info("Chain progressed: slot %d -> %d", initial_slot, updated_slot)

        assert updated_slot > initial_slot, (
            f"Chain did not progress: slot {initial_slot} -> {updated_slot}"
        )

        return True
