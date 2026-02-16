"""Test that EVM blocks are being generated."""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestBlockGeneration(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")

        initial_block = sequencer.get_block_number()
        logger.info(f"Initial block number: {initial_block}")

        for i in range(5):
            target_block = initial_block + i + 1
            sequencer.wait_for_block(target_block, timeout=30)
            current_block = sequencer.get_block_number()
            logger.info(f"Block {current_block} reached (target was {target_block})")

        final_block = sequencer.get_block_number()
        logger.info(f"Final block number: {final_block}")

        assert final_block >= initial_block + 5, (
            f"Expected at least {initial_block + 5} blocks, got {final_block}"
        )

        logger.info("Block generation test passed")
        return True
