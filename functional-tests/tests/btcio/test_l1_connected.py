"""Test that strata is connected to Bitcoin and tracking L1 blocks."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig

logger = logging.getLogger(__name__)


@flexitest.register
class TestL1Connected(StrataNodeTest):
    """Verify strata can see L1 blocks.

    A standalone env is used (not the shared "basic" env) so the bitcoin tip
    we read here is guaranteed to equal the genesis L1 height: nothing else
    has had a chance to mine more blocks or restart strata between env init
    and the start of this test. On the shared "basic" env, sibling tests
    (e.g. test_sequencer_restart) can advance the bitcoin tip past where
    strata's L1 reader has caught up after a restart, causing this test to
    flake. The other btcio tests use the same standalone-env pattern; see
    test_l1_tracking.py and test_l1_reorg.py.

    Replaces old: btcio_connect.py (strata_l1connected)
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110))

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()

        # In a standalone env, the current bitcoin tip equals the genesis L1
        # height: it was set during env init and nothing has touched bitcoin
        # since. The ASM only creates manifests for heights >= genesis, so
        # checking the genesis height itself proves the L1 reader is connected
        # and processing blocks.
        chain_info = btc_rpc.proxy.getblockchaininfo()
        genesis_l1_height = chain_info["blocks"]
        logger.info(f"Genesis L1 height: {genesis_l1_height}")

        commitment = strata.wait_for_l1_commitment_at(genesis_l1_height, rpc=rpc, timeout=60)

        logger.info(f"L1 header commitment at {genesis_l1_height}: {commitment}")
        return True
