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
        final_height = strata.wait_for_additional_blocks(blocks_to_produce, rpc)
        produced_blocks = final_height - initial_height

        if produced_blocks < blocks_to_produce:
            raise AssertionError(
                f"Expected at least {blocks_to_produce} new blocks, got {produced_blocks}",
            )

        logger.info(
            "Sequencer produced %s new blocks successfully (height %s -> %s)",
            produced_blocks,
            initial_height,
            final_height,
        )
        return True
