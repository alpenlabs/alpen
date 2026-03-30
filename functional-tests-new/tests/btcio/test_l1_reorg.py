"""Test that strata handles Bitcoin L1 chain reorganizations."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig

logger = logging.getLogger(__name__)

# How many blocks above genesis to mine before triggering reorg.
EXTRA_BLOCKS = 6
# How many of those extra blocks to invalidate.
REORG_DEPTH = 3


@flexitest.register
class TestL1Reorg(StrataNodeTest):
    """Verify strata detects and handles L1 block reorganizations.

    Mines blocks above genesis so the ASM has manifests to compare,
    then invalidates some of those blocks, mines replacements, and
    checks that strata updates its L1 header commitments.

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

        # Genesis L1 height = current bitcoin tip (set during env init).
        # The ASM only creates manifests for heights >= genesis, so we must
        # mine additional blocks and reorg within *those*, not below genesis.
        genesis_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        logger.info(f"Genesis L1 tip: {genesis_tip}")

        # Mine blocks above genesis one at a time so the ASM processes each
        # before the next arrives (avoids L1 reader / ASM notification race).
        addr = btc_rpc.proxy.getnewaddress()
        for _ in range(EXTRA_BLOCKS):
            btc_rpc.proxy.generatetoaddress(1, addr)
        tip_height = btc_rpc.proxy.getblockchaininfo()["blocks"]
        logger.info(f"Bitcoin tip after extra mining: {tip_height}")

        # Pick a height to invalidate — must be above genesis.
        invalidate_height = tip_height - REORG_DEPTH
        assert invalidate_height > genesis_tip, (
            f"invalidate_height {invalidate_height} must be above genesis {genesis_tip}"
        )
        logger.info(f"Will invalidate from height {invalidate_height}")

        # Wait for strata to have processed the block at this height.
        pre_reorg_commitment = strata.wait_for_l1_commitment(
            invalidate_height, rpc=rpc, timeout=120
        )
        logger.info(f"Pre-reorg commitment at {invalidate_height}: {pre_reorg_commitment}")

        # Invalidate the block (and all descendants).
        block_hash = btc_rpc.proxy.getblockhash(invalidate_height)
        logger.info(f"Invalidating block {block_hash}")
        btc_rpc.proxy.invalidateblock(block_hash)

        # Sanity check: bitcoin tip should have regressed.
        regressed_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        if regressed_tip >= invalidate_height:
            raise AssertionError(
                f"Expected tip below {invalidate_height} after invalidation, got {regressed_tip}"
            )
        logger.info(f"Bitcoin tip regressed to {regressed_tip}")

        # Mine replacement blocks past the old invalidation point one at a time.
        blocks_to_mine = REORG_DEPTH + 2
        for _ in range(blocks_to_mine):
            btc_rpc.proxy.generatetoaddress(1, addr)
        post_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        logger.info(f"Post-reorg Bitcoin tip: {post_tip}")

        # Wait for strata to pick up the new chain; the commitment
        # at invalidate_height must differ from the pre-reorg value.
        post_reorg_commitment = strata.wait_for_l1_commitment(
            invalidate_height,
            rpc=rpc,
            timeout=120,
            differs_from=pre_reorg_commitment,
        )
        logger.info(f"Post-reorg commitment at {invalidate_height}: {post_reorg_commitment}")

        logger.info(
            "Strata detected L1 reorg: commitment changed at height %d",
            invalidate_height,
        )
        return True
