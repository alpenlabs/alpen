"""Test sequencer recovers after crash during fork choice new block processing."""

import logging

import flexitest

from common.crash_helpers import CrashTest, crash_and_recover
from tests.dbtool.helpers import assert_ol_block_status, get_ol_blocks_at_slot

logger = logging.getLogger(__name__)


def get_single_block_at_slot(datadir: str, slot: int) -> str:
    blocks_at_slot = get_ol_blocks_at_slot(datadir, slot)
    count = int(blocks_at_slot["count"])
    block_ids = blocks_at_slot["block_ids"]

    assert int(blocks_at_slot["slot"]) == slot
    assert count == 1, f"expected one OL block at slot {slot}, got {count}: {block_ids}"
    return block_ids[0]


def assert_single_block_at_slot(datadir: str, slot: int, expected_block_id: str) -> None:
    block_id = get_single_block_at_slot(datadir, slot)

    assert block_id == expected_block_id, (
        f"expected slot {slot} to contain only {expected_block_id}, got {block_id}"
    )


@flexitest.register
class TestCrashFcmNewBlock(CrashTest):
    """Crash at fcm_new_block bail point and verify recovery."""

    def main(self, ctx):
        strata = self.get_strata()
        rpc = strata.wait_for_rpc_ready(timeout=10)
        datadir = strata.props["datadir"]
        crashed_block: dict[str, int | str] = {}

        strata.wait_for_additional_blocks(2, rpc)

        def inspect_crashed_block(pre_status: dict) -> None:
            slot = int(pre_status["tip"]["slot"]) + 1
            block_id = get_single_block_at_slot(datadir, slot)
            block = assert_ol_block_status(datadir, block_id, "Unchecked")
            assert int(block["header_slot"]) == slot

            crashed_block["slot"] = slot
            crashed_block["block_id"] = block_id
            logger.info("Crashed block persisted unchecked: slot=%s block_id=%s", slot, block_id)

        # FCM crash happens during block processing — require 2 blocks of advance
        # to confirm the chain is genuinely producing again, not just one in flight.
        result = crash_and_recover(
            strata,
            "fcm_new_block",
            expected_block_advance=2,
            after_crash=inspect_crashed_block,
        )

        slot = int(crashed_block["slot"])
        block_id = str(crashed_block["block_id"])
        assert result.post_status["tip"]["slot"] > slot

        logger.info(f"Post-recovery height: {result.post_status['tip']['slot']}")
        strata.stop()

        assert_single_block_at_slot(datadir, slot, block_id)
        assert_ol_block_status(datadir, block_id, "Valid")
        logger.info("Recovered crashed block is valid and unique at slot=%s", slot)
        return True
