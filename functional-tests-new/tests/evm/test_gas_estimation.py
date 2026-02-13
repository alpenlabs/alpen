"""
Test that verifies gas estimation works correctly for various transaction types.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest
from common.config.constants import DEV_ADDRESS

logger = logging.getLogger(__name__)


@flexitest.register
class TestGasEstimation(AlpenClientTest):
    """
    Test gas estimation for different transaction types.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        rpc = sequencer.create_rpc()

        # Test 1: Simple ETH transfer
        gas = rpc.eth_estimateGas(
            {
                "from": DEV_ADDRESS,
                "to": "0x0000000000000000000000000000000000000001",
                "value": "0x1",
            }
        )
        gas_int = int(gas, 16)
        logger.info(f"Simple transfer gas estimate: {gas_int}")
        assert 21000 <= gas_int < 30000, f"Unexpected gas for simple transfer: {gas_int}"

        # Test 2: Transfer with data (should cost more)
        gas_with_data = rpc.eth_estimateGas(
            {
                "from": DEV_ADDRESS,
                "to": "0x0000000000000000000000000000000000000001",
                "value": "0x1",
                "data": "0x" + "ab" * 100,  # 100 bytes of data
            }
        )
        gas_with_data_int = int(gas_with_data, 16)
        logger.info(f"Transfer with data gas estimate: {gas_with_data_int}")
        assert gas_with_data_int > gas_int, "Data should increase gas cost"

        # Test 3: Zero value call
        gas_zero = rpc.eth_estimateGas(
            {
                "from": DEV_ADDRESS,
                "to": "0x0000000000000000000000000000000000000001",
                "value": "0x0",
            }
        )
        gas_zero_int = int(gas_zero, 16)
        logger.info(f"Zero value transfer gas estimate: {gas_zero_int}")
        assert gas_zero_int >= 21000, f"Unexpected gas for zero transfer: {gas_zero_int}"

        # Test 4: Estimate with specific block tag
        for tag in ["latest", "pending"]:
            gas_tag = rpc.eth_estimateGas(
                {
                    "from": DEV_ADDRESS,
                    "to": "0x0000000000000000000000000000000000000001",
                    "value": "0x1",
                },
                tag,
            )
            gas_tag_int = int(gas_tag, 16)
            logger.info(f"Gas estimate at '{tag}': {gas_tag_int}")
            assert gas_tag_int >= 21000, f"Unexpected gas at {tag}: {gas_tag_int}"

        logger.info("Gas estimation test passed")
        return True
