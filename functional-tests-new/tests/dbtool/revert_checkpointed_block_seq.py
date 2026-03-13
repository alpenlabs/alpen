"""Sequencer checkpointed-block revert with -c should succeed."""

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
    target_start_of_checkpointed_epoch,
    verify_checkpoint_deleted,
    verify_tip_resumed_with_new_blkid,
)

logger = logging.getLogger(__name__)


@flexitest.register
class RevertCheckpointedBlockSeqTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        logger.info("Starting sequencer checkpointed-block revert test")
        seq_service = self.get_service(ServiceType.Strata)
        btc_service = self.get_service(ServiceType.Bitcoin)
        setup = setup_revert_ol_state_test(seq_service, btc_service)
        seq_rpc = setup["rpc"]

        # Wait for extra blocks beyond checkpoint terminal.
        seq_service.wait_for_additional_blocks(5, seq_rpc, timeout_per_block=10)

        live_sync = seq_service.get_sync_status(seq_rpc)
        old_live_tip = live_sync["tip"]["slot"]
        old_live_blkid = live_sync["tip"]["blkid"]
        logger.info("Pre-revert live tip: slot=%s blkid=%s", old_live_tip, old_live_blkid)
        seq_service.stop()

        datadir = seq_service.props["datadir"]
        epoch_sealing = seq_service.props.get("epoch_sealing")
        if not isinstance(epoch_sealing, dict):
            raise AssertionError("Missing epoch_sealing config in sequencer service props")
        slots_per_epoch = epoch_sealing.get("slots_per_epoch")
        if not isinstance(slots_per_epoch, int) or slots_per_epoch <= 0:
            raise AssertionError(f"Invalid slots_per_epoch in sequencer props: {slots_per_epoch!r}")

        latest_checkpoint = get_latest_checkpoint(datadir)
        latest_epoch_before_revert = int(latest_checkpoint["checkpoint_epoch"])
        post_restart_target_epoch = latest_epoch_before_revert + 1
        epoch_summary_before = run_dbtool_json(
            datadir, "get-epoch-summary", str(latest_epoch_before_revert)
        )
        logger.info("Epoch summary before revert: %s", epoch_summary_before)
        target_block_id, _ = target_start_of_checkpointed_epoch(
            datadir,
            latest_checkpoint,
            slots_per_epoch,
        )
        target_before = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        logger.info("Executing revert -f -c to target block_id=%s", target_block_id)

        code, stdout, stderr = revert_ol_state(
            datadir,
            target_block_id,
            revert_checkpointed=True,
        )
        assert code == 0, stderr or stdout

        target_after = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        assert target_after["current_slot"] == target_before["current_slot"]
        logger.info("target OL state slot preserved after revert")
        # Reverting to start of checkpointed epoch should drop that epoch's checkpoint metadata.
        assert verify_checkpoint_deleted(datadir, latest_epoch_before_revert)

        # Restart and verify chain resumes and reorgs past old tip.
        seq_rpc, resumed_tip = restart_sequencer_after_revert(
            seq_service,
            old_live_tip,
            target_epoch=post_restart_target_epoch,
            error_with="Sequencer did not resume after checkpointed revert",
        )
        resumed_sync = verify_tip_resumed_with_new_blkid(
            seq_service,
            seq_rpc,
            old_live_tip,
            old_live_blkid,
            resumed_tip,
        )
        logger.info(
            "Chain resumed past old tip (old=%s new=%s) with new tip blkid=%s",
            old_live_tip,
            resumed_tip,
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
