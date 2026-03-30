"""Test that strata tracks new L1 blocks as they are mined."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig

logger = logging.getLogger(__name__)


@flexitest.register
class TestL1Tracking(StrataNodeTest):
    """Verify strata's L1 reader picks up newly mined Bitcoin blocks.

    Mines additional Bitcoin blocks after strata is running and verifies
    that strata_getL1HeaderCommitment returns data for the new heights.

    Uses a standalone env to avoid interference from other tests that
    may restart the sequencer.

    Replaces old: btcio_read.py (strata_l1status)
    """

    EXTRA_BLOCKS = 5

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110))

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()

        # Record the current Bitcoin tip
        pre_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        logger.info(f"Bitcoin tip before mining: {pre_tip}")

        # Wait for strata to have caught up to pre_tip
        strata.wait_for_l1_commitment(pre_tip, rpc=rpc, timeout=120)

        # Mine additional blocks one at a time.  Mining in a single batch can
        # trigger a race in the L1 reader / ASM pipeline where the ASM is
        # notified about a block before the full block data is persisted.
        addr = btc_rpc.proxy.getnewaddress()
        for _ in range(self.EXTRA_BLOCKS):
            btc_rpc.proxy.generatetoaddress(1, addr)
        post_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        logger.info(f"Bitcoin tip after mining {self.EXTRA_BLOCKS} blocks: {post_tip}")

        if post_tip != pre_tip + self.EXTRA_BLOCKS:
            raise AssertionError(f"Expected tip {pre_tip + self.EXTRA_BLOCKS}, got {post_tip}")

        # Wait for strata to pick up the new blocks
        commitment = strata.wait_for_l1_commitment(post_tip, rpc=rpc, timeout=120)
        logger.info(f"L1 header commitment at new tip {post_tip}: {commitment}")

        # Verify intermediate heights also have commitments
        for h in range(pre_tip + 1, post_tip + 1):
            c = rpc.strata_getL1HeaderCommitment(h)
            if c is None:
                raise AssertionError(f"Missing L1 header commitment at height {h}")

        logger.info(
            "Strata tracked all %d new L1 blocks (%d -> %d)",
            self.EXTRA_BLOCKS,
            pre_tip,
            post_tip,
        )
        return True
