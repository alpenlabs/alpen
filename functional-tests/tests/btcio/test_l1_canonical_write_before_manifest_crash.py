"""Test restart after crashing between L1 canonical write and ASM notification."""

import logging

import flexitest

from common.bail_tags import require_known_bail_tag
from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig

logger = logging.getLogger(__name__)

@flexitest.register
class TestL1CanonicalWriteBeforeManifestCrash(StrataNodeTest):
    """Verify a canonical L1 entry without a manifest is recovered on restart."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110))

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        bitcoin = self.get_service(ServiceType.Bitcoin)

        rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()
        mine_addr = btc_rpc.proxy.getnewaddress()

        btc_rpc.proxy.generatetoaddress(1, mine_addr)
        initial_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        strata.wait_for_asm_manifest_commitment_at(initial_tip, rpc=rpc, timeout=180)
        logger.info("Initial L1 tip %d is tracked", initial_tip)

        bail_tag = require_known_bail_tag(rpc, "btcio_after_l1_canonical_write")
        rpc.debug_bail(bail_tag)

        btc_rpc.proxy.generatetoaddress(1, mine_addr)
        partial_height = btc_rpc.proxy.getblockchaininfo()["blocks"]
        if partial_height != initial_tip + 1:
            raise AssertionError(f"Expected partial height {initial_tip + 1}, got {partial_height}")

        strata.wait_for_down(timeout=30)

        # ProcService bookkeeping still thinks the process is running after an
        # abort, so clear it before restart.
        strata.stop()
        strata.start()
        rpc = strata.wait_for_rpc_ready(timeout=30)

        partial_commitment = strata.wait_for_asm_manifest_commitment_at(
            partial_height, rpc=rpc, timeout=60
        )

        btc_rpc.proxy.generatetoaddress(1, mine_addr)
        final_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        strata.wait_for_asm_manifest_commitment_at(final_tip, rpc=rpc, timeout=180)
        if not strata.check_status():
            raise AssertionError("Strata crashed after partial L1 canonical write recovery")

        logger.info(
            "Strata recovered partial L1 canonical write at height %d commitment %s "
            "and processed L1 height %d",
            partial_height,
            partial_commitment,
            final_tip,
        )
        return True
