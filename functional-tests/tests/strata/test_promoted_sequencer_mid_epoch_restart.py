"""A promoted sequencer resumes after a non-terminal mid-epoch restart."""

import logging

import flexitest

from common.base_test import BaseTest
from common.wait import wait_until_with_value
from envconfigs.el_ol_checkpoint_sync import EeOLCheckpointSyncEnv
from tests.strata.checkpoint_promotion import (
    finalize_active_checkpoint,
    finalize_promoted_epoch,
    nuke_sequencer_and_promote,
)

logger = logging.getLogger(__name__)


@flexitest.register
class TestPromotedSequencerMidEpochRestart(BaseTest):
    """Restarts the promoted service mid-epoch and finalizes that epoch."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            EeOLCheckpointSyncEnv(
                pre_generate_blocks=110,
                seal_epoch_slots=4,
                ol_block_time_ms=1_000,
                l1_reorg_safe_depth=4,
                batch_sealing_block_count=3,
                provision_promotion=True,
            )
        )

    def main(self, ctx):
        anchor = finalize_active_checkpoint(self)
        promoted = nuke_sequencer_and_promote(self, anchor)

        pre_restart_tip = wait_until_with_value(
            lambda: promoted.service.get_sync_status(promoted.rpc)["tip"],
            lambda tip: tip["slot"] > anchor.slot and not tip["is_terminal"],
            error_with="promoted sequencer did not produce a non-terminal post-anchor block",
            timeout=60,
            step=0.05,
        )
        logger.info(
            "restarting promoted sequencer mid-epoch at epoch=%s slot=%s",
            pre_restart_tip["epoch"],
            pre_restart_tip["slot"],
        )
        promoted.signer.stop()
        promoted.service.stop()
        assert "--bootstrap-from-checkpoint" in promoted.service.cmd
        promoted.service.start()
        promoted.rpc = promoted.service.wait_for_rpc_ready(timeout=60)
        promoted.signer.start()
        promoted.signer.wait_for_ready(timeout=10)

        promoted.service.wait_for_block_height(
            pre_restart_tip["slot"] + 1,
            promoted.rpc,
            timeout=60,
            poll_interval=0.2,
        )
        next_block = promoted.rpc.strata_getBlockBySlot(pre_restart_tip["slot"] + 1)
        assert next_block is not None
        assert next_block["header"]["parent_blkid"] == pre_restart_tip["blkid"]

        finalize_promoted_epoch(
            self,
            anchor,
            promoted,
            target_epoch=pre_restart_tip["epoch"],
        )
        return True
