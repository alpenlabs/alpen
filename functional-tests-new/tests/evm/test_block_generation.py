"""Test that EVM blocks are being generated."""

import logging

import flexitest

from common.base_test import AlpenClientTest
from common.config.constants import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestBlockGeneration(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        ee_sequencer = self.get_service(ServiceType.AlpenSequencer)

        initial_block = ee_sequencer.get_block_number()
        logger.info(f"Initial block number: {initial_block}")

        target_block = initial_block + 5
        ee_sequencer.wait_for_additional_blocks(5)

        final_block = ee_sequencer.get_block_number()
        logger.info(f"Final block number: {final_block}")

        assert final_block >= target_block, (
            f"Expected at least block {target_block}, got {final_block}"
        )

        logger.info("Block generation test passed")
        return True
