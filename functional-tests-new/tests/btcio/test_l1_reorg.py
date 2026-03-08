"""Test that strata handles Bitcoin L1 chain reorganizations."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.wait import wait_until_with_value
from envconfigs.strata import StrataEnvConfig

logger = logging.getLogger(__name__)

REORG_DEPTH = 3


@flexitest.register
class TestL1Reorg(StrataNodeTest):
    """Verify strata detects and handles L1 block reorganizations.

    Invalidates Bitcoin blocks to trigger a chain reorg, then mines
    replacement blocks and checks that strata updates its L1 header
    commitments at the affected heights.

    Replaces old: btcio_read_reorg.py (L1ReadReorgTest)
    """

    def __init__(self, ctx: flexitest.InitContext):
        # standalone env: this test mutates the bitcoin chain via invalidateblock
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110))

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()

        # get current bitcoin tip
        tip_height = btc_rpc.proxy.getblockchaininfo()["blocks"]
        logger.info(f"Bitcoin tip: {tip_height}")

        # pick the height to invalidate from
        invalidate_height = tip_height - REORG_DEPTH
        logger.info(f"Will invalidate from height {invalidate_height}")

        # wait for strata to have processed the block at this height
        pre_reorg_commitment = wait_until_with_value(
            lambda: rpc.strata_getL1HeaderCommitment(invalidate_height),
            lambda v: v is not None,
            timeout=30,
            error_with=f"Strata not caught up to height {invalidate_height}",
        )
        logger.info(
            f"Pre-reorg commitment at {invalidate_height}: {pre_reorg_commitment}"
        )

        # invalidate the block (and all descendants)
        block_hash = btc_rpc.proxy.getblockhash(invalidate_height)
        logger.info(f"Invalidating block {block_hash}")
        btc_rpc.proxy.invalidateblock(block_hash)

        # sanity check: bitcoin tip should have regressed
        regressed_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        if regressed_tip >= invalidate_height:
            raise AssertionError(
                f"Expected tip below {invalidate_height} after invalidation, "
                f"got {regressed_tip}"
            )
        logger.info(f"Bitcoin tip regressed to {regressed_tip}")

        # mine replacement blocks past the old invalidation point
        addr = btc_rpc.proxy.getnewaddress()
        blocks_to_mine = REORG_DEPTH + 2
        btc_rpc.proxy.generatetoaddress(blocks_to_mine, addr)
        post_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        logger.info(f"Post-reorg Bitcoin tip: {post_tip}")

        # wait for strata to pick up the new chain; the commitment
        # at invalidate_height must differ from the pre-reorg value
        post_reorg_commitment = wait_until_with_value(
            lambda: rpc.strata_getL1HeaderCommitment(invalidate_height),
            lambda v: v is not None and v != pre_reorg_commitment,
            timeout=30,
            error_with=(
                f"Strata did not update commitment at height "
                f"{invalidate_height} after reorg"
            ),
        )
        logger.info(
            f"Post-reorg commitment at {invalidate_height}: {post_reorg_commitment}"
        )

        logger.info(
            "Strata detected L1 reorg: commitment changed at height %d",
            invalidate_height,
        )
        return True
