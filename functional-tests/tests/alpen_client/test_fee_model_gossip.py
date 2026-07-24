"""Test fee-model constant propagation over Alpen gossip."""

import logging

import flexitest

from common.base_test import AlpenClientTest
from common.config import FeeModelConfig
from common.config.constants import ServiceType
from common.wait import wait_until
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)

EXPECTED_FEE_CONFIG = {
    "prover_fee_per_gas_wei": 25,
    "da_overhead_multiplier_bps": 12_500,
    "ol_overhead_wei": 42,
}


@flexitest.register
class TestFeeModelGossip(AlpenClientTest):
    """Sequencer gossips static fee constants to a fullnode."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            AlpenClientEnv(
                enable_l1_da=True,
                fee_model=FeeModelConfig(
                    prover_fee_per_gas_wei=EXPECTED_FEE_CONFIG["prover_fee_per_gas_wei"],
                    da_overhead_multiplier_bps=EXPECTED_FEE_CONFIG["da_overhead_multiplier_bps"],
                    ol_overhead_wei=EXPECTED_FEE_CONFIG["ol_overhead_wei"],
                ),
            )
        )

    def main(self, ctx):
        sequencer = self.get_service(ServiceType.AlpenSequencer)
        fullnode = self.get_service(ServiceType.AlpenFullNode)

        sequencer.wait_for_peers(1, timeout=60)
        fullnode.wait_for_peers(1, timeout=60)

        sequencer_config = sequencer.get_fee_model_config()
        assert sequencer_config == EXPECTED_FEE_CONFIG, sequencer_config

        target_block = sequencer.wait_for_additional_blocks(1)
        fullnode.wait_for_block(target_block, timeout=60)

        wait_until(
            lambda: fullnode.get_fee_model_config() == EXPECTED_FEE_CONFIG,
            error_with="fullnode did not apply gossiped fee-model config",
            timeout=60,
        )

        logger.info("Fee config propagated to fullnode: %s", EXPECTED_FEE_CONFIG)
        return True
