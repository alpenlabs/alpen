"""Test the EVM genesis block hash."""

import logging

import flexitest

from common.base_test import AlpenClientTest
from common.config.constants import ServiceType

logger = logging.getLogger(__name__)

EXPECTED_GENESIS_HASH = "0x5c17d78b1ce8c2c52fb850077d0056864f334f3b619d2c4a5e15ab9709710f4f"


@flexitest.register
class TestGenesisBlockHash(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        ee_sequencer = self.get_service(ServiceType.AlpenSequencer)
        rpc = ee_sequencer.create_rpc()

        genesis_block = rpc.eth_getBlockByNumber("0x0", False)

        actual_hash = genesis_block["hash"]
        logger.info(f"Genesis block hash: {actual_hash}")

        assert actual_hash == EXPECTED_GENESIS_HASH, (
            f"Genesis block hash mismatch.\n"
            f"Expected: {EXPECTED_GENESIS_HASH}\n"
            f"Actual:   {actual_hash}"
        )

        logger.info("Genesis block hash verified successfully")
        return True
