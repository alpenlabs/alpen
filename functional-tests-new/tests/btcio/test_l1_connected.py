"""Test that strata is connected to Bitcoin and tracking L1 blocks."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType

logger = logging.getLogger(__name__)


@flexitest.register
class TestL1Connected(StrataNodeTest):
    """Verify strata can see L1 blocks.

    The basic env pre-generates 110 Bitcoin blocks before starting strata.
    After strata starts, it should have L1 header commitments for those
    blocks. We check that the genesis L1 height has a commitment, which
    proves the L1 reader is connected and processing blocks.

    Replaces old: btcio_connect.py (strata_l1connected)
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()

        # The basic env pre-generates 110 blocks. The genesis L1 height
        # is the tip at the time strata started (~110). Pick a height
        # we know exists and wait for strata to have a commitment for it.
        chain_info = btc_rpc.proxy.getblockchaininfo()
        tip_height = chain_info["blocks"]
        logger.info(f"Bitcoin tip: {tip_height}")

        # Wait for strata to have processed at least the genesis L1 block.
        # Use a height slightly below the tip to avoid races.
        check_height = tip_height - 2
        logger.info(f"Checking L1 header commitment at height {check_height}")

        commitment = strata.wait_for_l1_commitment_at(check_height, rpc=rpc, timeout=30)

        logger.info(f"L1 header commitment at {check_height}: {commitment}")
        return True
