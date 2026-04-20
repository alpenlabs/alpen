"""Test sequencer block production and checkpoint finalization with CredRule::Unchecked."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from common.wait import wait_until_with_value
from envconfigs.strata_unchecked import StrataUncheckedEnvConfig
from tests.checkpoint.helpers import mine_until_finalized_epoch

logger = logging.getLogger(__name__)


@flexitest.register
class TestSequencerUnchecked(StrataNodeTest):
    """Verify block production and checkpoint finalization without an external signer.

    Uses ``CredRule::Unchecked`` — no strata-signer process is needed.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataUncheckedEnvConfig(pre_generate_blocks=110))

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)

        rpc = strata.wait_for_rpc_ready(timeout=30)

        wait_until_with_value(
            rpc.strata_getChainStatus,
            lambda x: x is not None,
            error_with="Timed out waiting for OL chain status",
            timeout=60,
        )

        initial_height = strata.get_cur_block_height(rpc)
        logger.info("initial block height: %s", initial_height)

        blocks_to_produce = 4
        final_height = strata.wait_for_additional_blocks(blocks_to_produce, rpc)
        produced_blocks = final_height - initial_height

        if produced_blocks < blocks_to_produce:
            raise AssertionError(
                f"Expected at least {blocks_to_produce} new blocks, got {produced_blocks}",
            )

        logger.info(
            "sequencer produced %s new blocks with CredRule::Unchecked (height %s -> %s)",
            produced_blocks,
            initial_height,
            final_height,
        )

        epoch = mine_until_finalized_epoch(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=rpc,
            target_epoch=1,
            timeout=60,
            step=1.0,
        )
        logger.info("finalized epoch advanced to %s", epoch["epoch"])

        return True
