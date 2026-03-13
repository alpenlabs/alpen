"""Sequencer revert-ol-state with -d should delete reverted blocks."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import (
    get_latest_checkpoint,
    restart_sequencer_after_revert,
    revert_ol_state,
    run_dbtool,
    run_dbtool_json,
    setup_revert_ol_state_test,
    target_end_of_checkpointed_epoch,
    verify_checkpoint_preserved,
    verify_tip_resumed_with_new_blkid,
    wait_for_finalized_epoch_with_mining,
)

logger = logging.getLogger(__name__)


@flexitest.register
class RevertOLStateDeleteBlocksTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        logger.info("Starting delete-blocks revert test (with override)")
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
        target_finalized_epoch = latest_epoch_before_revert + 1
        target_block_id, target_slot = target_end_of_checkpointed_epoch(latest_checkpoint)
        target_before = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        assert int(target_before["current_slot"]) == target_slot
        logger.info("Target slot: %s, target block ID: %s", target_slot, target_block_id)
        logger.info("Testing revert-ol-state to %s with -d flag", target_block_id)

        code, stdout, stderr = revert_ol_state(datadir, target_block_id, delete_blocks=True)
        assert code == 0, stderr or stdout
        target_after = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        assert int(target_after["current_slot"]) == target_slot
        logger.info("target OL state preserved after revert")
        assert verify_checkpoint_preserved(datadir, latest_epoch_before_revert)
        logger.info("checkpoint and epoch summary are preserved")
        deleted_code, _, _ = run_dbtool(datadir, "get-ol-block", old_live_tip_blkid, "-o", "json")
        assert deleted_code != 0
        logger.info("Confirmed reverted tip block was deleted: %s", old_live_tip_blkid)

        # Restart and verify chain resumes and reorgs past old tip.
        seq_rpc, resumed_slot = restart_sequencer_after_revert(
            seq_service,
            old_live_tip_slot,
            error_with="Sequencer did not resume block production after -d revert",
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
        wait_for_finalized_epoch_with_mining(
            seq_service,
            seq_rpc,
            setup["btc_rpc"],
            setup["btc_rpc"].proxy.getnewaddress(),
            target_epoch=target_finalized_epoch,
            timeout=180,
        )
        logger.info(
            "Criterion passed: finalized epoch reached "
            "latest checkpoint epoch before revert + 1 "
            "(latest_before_revert=%s target=%s)",
            latest_epoch_before_revert,
            target_finalized_epoch,
        )
        return True
