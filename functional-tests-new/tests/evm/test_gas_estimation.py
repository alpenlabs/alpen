"""Test gas estimation for transactions."""

import logging

import flexitest

from common.base_test import AlpenClientTest
from common.config.constants import DEV_ADDRESS

logger = logging.getLogger(__name__)

SIMPLE_TRANSFER_GAS = 21000


@flexitest.register
class TestGasEstimation(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        ee_sequencer = self.get_service("ee_sequencer")
        rpc = ee_sequencer.create_rpc()

        gas = rpc.eth_estimateGas(
            {
                "from": DEV_ADDRESS,
                "to": "0x000000000000000000000000000000000000dEaD",
                "value": "0x1",
            }
        )
        gas_int = int(gas, 16)
        logger.info(f"Simple transfer gas estimate: {gas_int}")
        assert gas_int == SIMPLE_TRANSFER_GAS, (
            f"Expected {SIMPLE_TRANSFER_GAS} gas for simple transfer, got {gas_int}"
        )

        gas_with_data = rpc.eth_estimateGas(
            {
                "from": DEV_ADDRESS,
                "to": "0x000000000000000000000000000000000000dEaD",
                "value": "0x1",
                "data": "0x" + "ab" * 100,
            }
        )
        gas_with_data_int = int(gas_with_data, 16)
        logger.info(f"Transfer with data gas estimate: {gas_with_data_int}")
        assert gas_with_data_int > SIMPLE_TRANSFER_GAS, "Data should increase gas cost"

        gas_zero = rpc.eth_estimateGas(
            {
                "from": DEV_ADDRESS,
                "to": "0x000000000000000000000000000000000000dEaD",
                "value": "0x0",
            }
        )
        gas_zero_int = int(gas_zero, 16)
        logger.info(f"Zero value transfer gas estimate: {gas_zero_int}")
        assert gas_zero_int == SIMPLE_TRANSFER_GAS, (
            f"Expected {SIMPLE_TRANSFER_GAS} gas for zero transfer, got {gas_zero_int}"
        )

        for tag in ["latest", "pending"]:
            gas_tag = rpc.eth_estimateGas(
                {
                    "from": DEV_ADDRESS,
                    "to": "0x000000000000000000000000000000000000dEaD",
                    "value": "0x1",
                },
                tag,
            )
            gas_tag_int = int(gas_tag, 16)
            logger.info(f"Gas estimate at '{tag}': {gas_tag_int}")
            assert gas_tag_int == SIMPLE_TRANSFER_GAS, (
                f"Expected {SIMPLE_TRANSFER_GAS} gas at {tag}, got {gas_tag_int}"
            )

        logger.info("Gas estimation test passed")
        return True
