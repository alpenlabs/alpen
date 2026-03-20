"""Test OL block query RPC methods (range and individual lookups)."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)

BLOCKS_TO_QUERY = 5


@flexitest.register
class TestOLBlockQueries(StrataNodeTest):
    """Test block range queries and individual block lookups."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        rpc = strata.wait_for_rpc_ready(timeout=10)

        # Wait for enough blocks to exist
        initial_height = strata.get_cur_block_height(rpc)
        target_height = initial_height + BLOCKS_TO_QUERY
        strata.wait_for_block_height(target_height, rpc, timeout=30)

        # Query a range of blocks (inclusive on both ends)
        start_slot = initial_height + 1
        end_slot = target_height
        blocks = rpc.strata_getRawBlocksRange(start_slot, end_slot)
        expected_count = end_slot - start_slot + 1

        logger.info(
            "Queried range [%d, %d]: got %d blocks (expected %d)",
            start_slot,
            end_slot,
            len(blocks),
            expected_count,
        )
        assert len(blocks) == expected_count, (
            f"Expected {expected_count} blocks in range [{start_slot}, {end_slot}], "
            f"got {len(blocks)}"
        )

        # Verify blocks have expected fields and sequential slots
        for i, block in enumerate(blocks):
            expected_slot = start_slot + i
            assert block["slot"] == expected_slot, (
                f"Block {i} has slot {block['slot']}, expected {expected_slot}"
            )
            assert block["blkid"], f"Block at slot {expected_slot} has empty blkid"
            assert block["raw_block"], f"Block at slot {expected_slot} has empty raw_block"

        # Verify block IDs are unique
        blkids = [b["blkid"] for b in blocks]
        assert len(set(blkids)) == len(blkids), "Duplicate block IDs in range"

        # Query individual block by ID and verify consistency with range data
        target_block = blocks[0]
        raw_block = rpc.strata_getRawBlockById(target_block["blkid"])
        assert raw_block is not None, "getRawBlockById returned None"
        assert raw_block == target_block["raw_block"], (
            "Block data from getRawBlockById doesn't match range query for "
            f"blkid {target_block['blkid'][:16]}..."
        )

        logger.info(
            "Verified %d blocks in range [%d, %d] with consistent individual lookup",
            len(blocks),
            start_slot,
            end_slot,
        )

        return True
