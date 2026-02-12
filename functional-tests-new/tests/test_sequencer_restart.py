"""Test sequencer continues producing blocks after restart."""

import logging
import time

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


@flexitest.register
class TestSequencerRestart(StrataNodeTest):
    """Test that sequencer resumes block production after restart."""

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
        initial_height = initial_status.get("latest", {}).get("slot", 0)
        logger.info(f"Initial block height: {initial_height}")

        # Wait for some blocks to be produced
        num_slots = 3
        for target_height in range(initial_height, initial_height + num_slots):
            # Wait for new blocks to be produced
            logger.info(f"Waiting for chain to reach height {target_height}...")

            wait_until_with_value(
                lambda: strata_rpc.strata_getChainStatus(),
                lambda status: status.get("latest", {}).get("slot", 0) >= target_height,
                error_with=f"Timeout waiting for block height {target_height}",
                timeout=10,
                step=1.0
            )

        pre_restart_height = strata_rpc.strata_getChainStatus().get("latest", {}).get("slot", 0)
        logger.info(f"Height before restart: {pre_restart_height}")

        # Restart the sequencer
        logger.info("Restarting Strata sequencer...")
        strata.stop()
        time.sleep(2)  # Brief pause to ensure clean shutdown
        strata.start()

        # Wait for RPC to be ready again
        logger.info("Waiting for Strata RPC to be ready after restart...")
        strata.wait_for_rpc_ready(timeout=20)

        num_slots = 3
        for target_height in range(pre_restart_height, pre_restart_height + num_slots):
            # Wait for new blocks to be produced
            logger.info(f"Waiting for chain to reach height {target_height}...")

            wait_until_with_value(
                lambda: strata_rpc.strata_getChainStatus(),
                lambda status: status.get("latest", {}).get("slot", 0) >= target_height,
                error_with=f"Timeout waiting for block height {target_height}",
                timeout=10,
                step=1.0
            )
        logger.info("Sequencer successfully resumed block production after restart!")
        return True
