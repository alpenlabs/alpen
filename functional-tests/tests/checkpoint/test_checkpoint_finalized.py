"""
Verify that signed checkpoints are posted to L1 and the ASM finalizes them.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from tests.checkpoint.helpers import mine_until_finalized_epoch

logger = logging.getLogger(__name__)

ZERO_HASH = "00" * 32


@flexitest.register
class TestCheckpointFinalized(StrataNodeTest):
    """Signed checkpoint posted to L1, ASM finalizes it, finalized epoch advances."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("checkpoint")

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)

        # Wait for RPC to be ready
        logger.info("Waiting for Strata RPC to be ready...")
        strata_rpc = strata.wait_for_rpc_ready(timeout=20)

        # Get initial sync status
        initial_status = strata.get_sync_status(strata_rpc)
        logger.info(
            "initial finalized cursor epoch %s (genesis baseline)",
            initial_status["finalized"]["epoch"],
        )

        epochs_to_check = 3

        for target_epoch in range(1, epochs_to_check + 1):
            epoch = mine_until_finalized_epoch(
                bitcoin=bitcoin,
                strata=strata,
                strata_rpc=strata_rpc,
                target_epoch=target_epoch,
                timeout=120,
                step=1.0,
            )
            logger.info("finalized epoch advanced to %s", epoch["epoch"])

            # The L1 reference returned by getCheckpointInfo must carry the real
            # txid/wtxid extracted by CSM, not the legacy zero placeholders.
            info = strata_rpc.call("strata_getCheckpointInfo", target_epoch)
            assert info is not None, f"no checkpoint info for finalized epoch {target_epoch}"
            status = info["confirmation_status"]
            assert status["status"] == "finalized", (
                f"epoch {target_epoch} not finalized in checkpoint info: {status}"
            )
            l1_ref = status["l1_reference"]
            assert l1_ref["txid"] != ZERO_HASH, (
                f"epoch {target_epoch} l1_reference has zero txid: {l1_ref}"
            )
            assert l1_ref["wtxid"] != ZERO_HASH, (
                f"epoch {target_epoch} l1_reference has zero wtxid: {l1_ref}"
            )

        return True
