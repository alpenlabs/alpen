"""Test that EVM blocks are being generated."""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestBlockGeneration(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        ee_sequencer = self.get_service("ee_sequencer")

        initial_block = ee_sequencer.get_block_number()
        logger.info(f"Initial block number: {initial_block}")

        target_block = initial_block + 5
        ee_sequencer.wait_for_block(target_block, timeout=10)

        final_block = ee_sequencer.get_block_number()
        logger.info(f"Final block number: {final_block}")

        assert final_block >= target_block, (
            f"Expected at least block {target_block}, got {final_block}"
        )

        logger.info("Block generation test passed")
        return True
