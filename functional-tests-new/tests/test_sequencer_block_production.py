"""Test sequencer block production."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


@flexitest.register
class TestSequencerBlockProduction(StrataNodeTest):
    """Test that sequencer produces blocks."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        # Get sequencer service
        strata = self.get_service(ServiceType.Strata)

        # Create RPC client
        strata_rpc = strata.create_rpc()

        logger.info("Waiting for Strata RPC to be ready...")
        strata.wait_for_rpc_ready(timeout=10)

        # Get initial chain status
        logger.info("Getting initial chain status...")
        initial_status = wait_until_with_value(
            strata_rpc.strata_getChainStatus,
            lambda x: x is not None,
            error_with="Timed out getting chain status",
        )
        initial_height = initial_status.get("latest", {}).get("height", 0)
        logger.info(f"Initial block height: {initial_height}")

        # Wait for new blocks to be produced
        target_height = initial_height + 5
        logger.info(f"Waiting for chain to reach height {target_height}...")

        final_status = wait_until_with_value(
            lambda: strata_rpc.strata_getChainStatus(),
            lambda status: status.get("latest", {}).get("height", 0) >= target_height,
            error_with=f"Timeout waiting for block height {target_height}",
            timeout=30,
            step=1.0
        )

        final_height = final_status.get("latest", {}).get("height", 0)
        blocks_produced = final_height - initial_height

        logger.info(f"Final block height: {final_height}")
        logger.info(f"Blocks produced: {blocks_produced}")

        # Verify blocks were produced
        assert blocks_produced >= 5, f"Expected at least 5 blocks, got {blocks_produced}"

        logger.info("Sequencer is producing blocks correctly!")
        return True
