"""Test block query RPC methods."""

import logging

import flexitest

from common.base_test import AlpenClientTest

logger = logging.getLogger(__name__)


@flexitest.register
class TestBlockQueries(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        ee_sequencer = self.get_service("ee_sequencer")
        rpc = ee_sequencer.create_rpc()

        ee_sequencer.wait_for_block(3, timeout=30)

        block_num = rpc.eth_blockNumber()
        block_num_int = int(block_num, 16)
        logger.info(f"Current block number: {block_num_int}")
        assert block_num_int >= 3, f"Expected at least 3 blocks, got {block_num_int}"

        for tag in ["earliest", "latest", "pending"]:
            block = rpc.eth_getBlockByNumber(tag, False)
            assert block is not None, f"Failed to get block at '{tag}'"
            logger.info(
                f"Block at '{tag}': number={block.get('number')}, hash={block.get('hash')[:18]}..."
            )

        block_0 = rpc.eth_getBlockByNumber("0x0", False)
        assert block_0 is not None, "Failed to get genesis block"
        assert block_0["number"] == "0x0", "Block number mismatch"
        logger.info(f"Genesis block hash: {block_0['hash']}")

        block_1 = rpc.eth_getBlockByNumber("0x1", False)
        assert block_1 is not None, "Failed to get block 1"
        assert block_1["parentHash"] == block_0["hash"], "Block 1 parent should be genesis"

        latest_block = rpc.eth_getBlockByNumber("latest", True)
        assert latest_block is not None, "Failed to get latest block with txs"
        assert "transactions" in latest_block, "Block should have transactions field"

        latest_hash = latest_block["hash"]
        block_by_hash = rpc.eth_getBlockByHash(latest_hash, False)
        assert block_by_hash is not None, f"Failed to get block by hash {latest_hash}"
        assert block_by_hash["hash"] == latest_hash, "Block hash mismatch"
        logger.info(f"Successfully queried block by hash: {latest_hash[:18]}...")

        future_block = rpc.eth_getBlockByNumber(hex(block_num_int + 1000), False)
        assert future_block is None, "Future block should not exist"

        tx_count = rpc.eth_getBlockTransactionCountByNumber("latest")
        tx_count_int = int(tx_count, 16)
        logger.info(f"Transaction count in latest block: {tx_count_int}")
        assert tx_count_int >= 0, "Transaction count should be non-negative"

        logger.info("Block queries test passed")
        return True
