"""Test to revert OL state in sequencer."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import (
    get_latest_checkpoint,
    restart_sequencer_after_revert,
    revert_ol_state,
    run_dbtool_json,
    setup_revert_ol_state_test,
    target_end_of_checkpointed_epoch,
    verify_checkpoint_preserved,
    verify_tip_resumed_with_new_blkid,
)

logger = logging.getLogger(__name__)


@flexitest.register
class RevertOLStateSeqTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        seq_service = self.get_service(ServiceType.Strata)
        btc_service = self.get_service(ServiceType.Bitcoin)
        setup = setup_revert_ol_state_test(seq_service, btc_service)
        seq_rpc = setup["rpc"]

        # Wait for extra blocks beyond checkpoint terminal.
        seq_service.wait_for_additional_blocks(5, seq_rpc, timeout_per_block=10)

        live_sync = seq_service.get_sync_status(seq_rpc)
        old_live_tip_slot = live_sync["tip"]["slot"]
        old_live_tip_blkid = live_sync["tip"]["blkid"]
        logger.info(
            "Pre-revert live tip: slot=%s blkid=%s",
            old_live_tip_slot,
            old_live_tip_blkid,
        )

        seq_service.stop()

        datadir = seq_service.props["datadir"]
        latest_checkpoint = get_latest_checkpoint(datadir)
        latest_epoch_before_revert = int(latest_checkpoint["checkpoint_epoch"])
        post_restart_target_epoch = latest_epoch_before_revert + 1
        target_block_id, target_slot = target_end_of_checkpointed_epoch(latest_checkpoint)

        old_tip_slot = run_dbtool_json(datadir, "get-syncinfo")["ol_tip_height"]
        target_state_before = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        logger.info(
            "Revert target selected: epoch=%s block_id=%s target_slot=%s",
            latest_epoch_before_revert,
            target_block_id,
            target_state_before["current_slot"],
        )

        code, stdout, stderr = revert_ol_state(datadir, target_block_id)
        assert code == 0, stderr or stdout

        after_sync = run_dbtool_json(datadir, "get-syncinfo")
        target_state_after = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        assert after_sync["ol_tip_height"] < old_tip_slot
        logger.info(
            "Tip moved back (before=%s after=%s)",
            old_tip_slot,
            after_sync["ol_tip_height"],
        )
        assert target_state_after["current_slot"] == target_state_before["current_slot"]
        logger.info("Target OL state slot is stable after revert")
        assert verify_checkpoint_preserved(datadir, latest_epoch_before_revert)
        logger.info("Checkpoint and epoch summary are preserved")

        # Restart and verify chain resumes and reorgs past old tip.
        seq_rpc, resumed_slot = restart_sequencer_after_revert(
            seq_service,
            old_live_tip_slot,
            target_epoch=post_restart_target_epoch,
            error_with="Sequencer did not resume block production after revert",
        )
        resumed_sync = verify_tip_resumed_with_new_blkid(
            seq_service,
            seq_rpc,
            old_live_tip_slot,
            old_live_tip_blkid,
            resumed_slot,
        )
        logger.info(
            "Chain resumed past old tip (old=%s new=%s) with new tip blkid=%s",
            old_live_tip_slot,
            resumed_slot,
            resumed_sync["tip"]["blkid"],
        )
        logger.info(
            "Criterion passed: checkpoint for latest checkpoint epoch "
            "before revert + 1 was created "
            "(latest_before_revert=%s target=%s)",
            latest_epoch_before_revert,
            post_restart_target_epoch,
        )
        return True
