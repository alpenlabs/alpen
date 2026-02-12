"""Test sequencer block production."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestSequencerBlockProduction(StrataNodeTest):
    """Test that sequencer produces blocks."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        # Get sequencer service
        strata = self.get_service(ServiceType.Strata)

        # Wait for RPC to be ready
        logger.info("Waiting for Strata RPC to be ready...")
        rpc = strata.wait_for_rpc_ready(timeout=10)

        # Get initial height
        initial_height = strata.get_cur_block_height(rpc)
        logger.info(f"Initial block height: {initial_height}")

        blocks_to_produce = 4
        cur_height = strata.check_block_generation_in_range(rpc, 1, blocks_to_produce)

        logger.info(f"Sequencer produced {cur_height} blocks successfully")
        return True
