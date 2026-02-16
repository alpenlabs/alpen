"""Test sequencer continues producing blocks after restart."""

import logging
import time

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.rpc import JsonRpcClient
from common.services.strata import StrataService

logger = logging.getLogger(__name__)


@flexitest.register
class TestSequencerRestart(StrataNodeTest):
    """Test that sequencer resumes block production after restart."""

    BLOCKS_BEFORE_RESTART = 3
    BLOCKS_AFTER_RESTART = 3
    RESTART_PAUSE_SECONDS = 2

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        # Get sequencer service
        strata = self.get_service(ServiceType.Strata)

        # Wait for RPC and create client
        logger.info("Waiting for Strata RPC to be ready...")
        rpc = strata.wait_for_rpc_ready(timeout=10)

        # Get initial height
        initial_height = strata.get_cur_block_height(rpc)
        logger.info(f"Initial block height: {initial_height}")

        # Wait for blocks before restart
        pre_restart_height = strata.wait_for_additional_blocks(
            self.BLOCKS_BEFORE_RESTART,
            rpc,
        )
        produced_before_restart = pre_restart_height - initial_height
        if produced_before_restart < self.BLOCKS_BEFORE_RESTART:
            raise AssertionError(
                "Expected at least "
                f"{self.BLOCKS_BEFORE_RESTART} new blocks before restart, "
                f"got {produced_before_restart}",
            )
        logger.info(f"Height before restart: {pre_restart_height}")

        rpc = self.restart_sequencer_and_get_rpc(strata)

        # Wait for blocks after restart
        final_height = strata.wait_for_additional_blocks(self.BLOCKS_AFTER_RESTART, rpc)
        produced_after_restart = final_height - pre_restart_height
        if produced_after_restart < self.BLOCKS_AFTER_RESTART:
            raise AssertionError(
                "Expected at least "
                f"{self.BLOCKS_AFTER_RESTART} new blocks after restart, "
                f"got {produced_after_restart}",
            )
        logger.info(f"Final height: {final_height}")
        logger.info("Sequencer successfully resumed block production after restart")
        return True

    def restart_sequencer_and_get_rpc(self, strata: StrataService) -> JsonRpcClient:
        # Restart the sequencer
        logger.info("Restarting Strata sequencer...")
        strata.stop()
        time.sleep(self.RESTART_PAUSE_SECONDS)  # Brief pause to ensure clean shutdown
        strata.start()

        # Wait for RPC to be ready again
        logger.info("Waiting for Strata RPC to be ready after restart...")
        return strata.wait_for_rpc_ready(timeout=20)
