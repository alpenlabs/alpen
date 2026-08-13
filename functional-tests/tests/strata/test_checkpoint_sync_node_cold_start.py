"""A fresh checkpoint-sync node backfills multiple finalized epochs from L1."""

import logging

import flexitest

from common.base_test import BaseTest
from common.checkpoint_sync import (
    check_summaries_equivalent,
    check_top_level_state_equivalent,
    mine_and_get_status,
)
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from envconfigs.checkpoint_sync import CheckpointSyncEnv

logger = logging.getLogger(__name__)

# The recovery node must reconstruct this many sealed epochs from an empty
# datadir, rather than following them live.
EPOCHS_TO_BACKFILL_FROM_COLD_START = 5


@flexitest.register
class TestCheckpointSyncNodeColdStart(BaseTest):
    """Starts an empty CSS node only after a multi-epoch checkpoint backlog exists."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            CheckpointSyncEnv(
                pre_generate_blocks=110,
                provision_recovery_node=True,
            )
        )

    def main(self, ctx):
        sequencer: StrataService = self.get_service(ServiceType.Strata)
        recovery_node: StrataService = self.get_service(ServiceType.StrataRecoveryCheckpointNode)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        sequencer.wait_for_rpc_ready(timeout=20)

        # The recovery node has not started yet, so its datadir contains no
        # synced OL state while the sequencer accumulates buried checkpoints.
        target_status = wait_until_with_value(
            lambda: mine_and_get_status(sequencer, btc_rpc),
            lambda status: status["finalized"]["epoch"] >= EPOCHS_TO_BACKFILL_FROM_COLD_START,
            error_with="sequencer did not finalize the cold-start checkpoint backlog",
            timeout=180,
        )
        target_epoch = target_status["finalized"]["epoch"]
        logger.info(f"starting empty checkpoint-sync node to recover through epoch {target_epoch}")

        recovery_node.start()
        recovery_node.wait_for_rpc_ready(timeout=30)

        seq_status, recovery_status = wait_until_with_value(
            lambda: (sequencer.get_sync_status(), recovery_node.get_sync_status()),
            lambda statuses: statuses[0]["finalized"] == statuses[1]["finalized"],
            error_with="cold checkpoint-sync node did not reconstruct the finalized backlog",
            timeout=180,
        )
        check_top_level_state_equivalent(seq_status, recovery_status)

        # Account summaries are immutable once finalized. Comparing every
        # reconstructed epoch catches a backfill that only lands on the final
        # checkpoint while skipping an intermediate epoch's state transition.
        for epoch in range(1, target_epoch + 1):
            seq_summary = sequencer.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch)
            recovery_summary = recovery_node.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch)
            check_summaries_equivalent(seq_summary, recovery_summary)
