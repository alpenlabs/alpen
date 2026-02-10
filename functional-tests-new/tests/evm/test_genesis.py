"""
Test that verifies the EVM genesis block hash.

This is a basic sanity test to ensure the EE is running with the expected
chain configuration.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)

# Expected genesis block hash for alpen-dev-chain
# This is deterministic based on the chain config in:
# crates/reth/chainspec/src/res/alpen-dev-chain.json
EXPECTED_GENESIS_HASH = "0x46c0dc60fb131be4ccc55306a345fcc20e44233324950f978ba5f185aa2af4dc"


@flexitest.register
class TestGenesisBlockHash(AlpenClientTest):
    """
    Verify the genesis block has the expected hash.

    This ensures the EE is initialized with the correct chain configuration.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        rpc = sequencer.create_rpc()

        # Fetch genesis block (block 0)
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
