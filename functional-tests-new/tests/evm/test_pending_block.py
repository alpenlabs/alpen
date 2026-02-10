"""
Test that verifies pending block queries and gas estimation work correctly.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest
from common.config.constants import DEV_ADDRESS

logger = logging.getLogger(__name__)


@flexitest.register
class TestPendingBlock(AlpenClientTest):
    """
    Verify pending block queries and gas estimation on pending block.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        rpc = sequencer.create_rpc()

        # Query pending block
        block = rpc.eth_getBlockByNumber("pending", True)
        assert block is not None, "Failed to get pending block"
        logger.info(f"Pending block number: {block.get('number')}")

        # Estimate gas on pending block
        gas = rpc.eth_estimateGas(
            {
                "from": DEV_ADDRESS,
                "to": "0x0000000000000000000000000000000000000001",
                "value": "0x1",
            },
            "pending",
        )

        assert gas is not None, "Failed to estimate gas on pending block"
        gas_int = int(gas, 16)
        logger.info(f"Estimated gas: {gas_int}")

        # Basic sanity check - simple transfer should be around 21000 gas
        assert gas_int >= 21000, f"Gas estimate too low: {gas_int}"
        assert gas_int < 100000, f"Gas estimate too high for simple transfer: {gas_int}"

        logger.info("Pending block test passed")
        return True
