"""Dry-run behavior for revert-ol-state (without --force)."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import (
    get_latest_checkpoint,
    ol_genesis_slot,
    revert_ol_state,
    run_dbtool_json,
    setup_revert_ol_state_test,
    target_start_of_checkpointed_epoch,
)

logger = logging.getLogger(__name__)


@flexitest.register
class RevertOLStateDryRunTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        logger.info("Starting revert dry-run test")
        seq_service = self.get_service(ServiceType.Strata)
        btc_service = self.get_service(ServiceType.Bitcoin)
        setup_revert_ol_state_test(seq_service, btc_service)

        seq_service.stop()
        datadir = seq_service.props["datadir"]
        epoch_sealing = seq_service.props.get("epoch_sealing")
        if not isinstance(epoch_sealing, dict):
            raise AssertionError("Missing epoch_sealing config in sequencer service props")
        slots_per_epoch = epoch_sealing.get("slots_per_epoch")
        if not isinstance(slots_per_epoch, int) or slots_per_epoch <= 0:
            raise AssertionError(f"Invalid slots_per_epoch in sequencer props: {slots_per_epoch!r}")

        latest_checkpoint = get_latest_checkpoint(datadir)
        latest_epoch = int(latest_checkpoint["checkpoint_epoch"])
        target_block_id, target_slot = target_start_of_checkpointed_epoch(
            datadir,
            latest_checkpoint,
            slots_per_epoch,
        )
        logger.info("Target slot: %s, target block ID: %s", target_slot, target_block_id)

        sync_before = run_dbtool_json(datadir, "get-syncinfo")
        tip_id = sync_before["ol_tip_block_id"]
        tip_slot = sync_before["ol_tip_height"]
        state_before = run_dbtool_json(datadir, "get-ol-state", tip_id)
        ol_summary_before = run_dbtool_json(datadir, "get-ol-summary", str(ol_genesis_slot()))
        checkpoints_before = run_dbtool_json(
            datadir,
            "get-checkpoints-summary",
            str(sync_before["l1_tip_height"]),
        )
        checkpoint_before = run_dbtool_json(datadir, "get-checkpoint", str(latest_epoch))
        assert checkpoint_before.get("checkpoint_epoch") is not None

        code, stdout, stderr = revert_ol_state(
            datadir,
            target_block_id,
            force=False,
            delete_blocks=True,
            revert_checkpointed=True,
        )
        assert code == 0, stderr or stdout
        assert "DRY RUN" in stdout
        logger.info("command executed in DRY RUN mode")

        sync_after = run_dbtool_json(datadir, "get-syncinfo")
        state_after = run_dbtool_json(datadir, "get-ol-state", tip_id)
        ol_summary_after = run_dbtool_json(datadir, "get-ol-summary", str(ol_genesis_slot()))
        checkpoints_after = run_dbtool_json(
            datadir,
            "get-checkpoints-summary",
            str(sync_after["l1_tip_height"]),
        )
        assert sync_after["ol_tip_block_id"] == tip_id
        assert sync_after["ol_tip_height"] == tip_slot
        assert state_after["current_slot"] == state_before["current_slot"]
        assert state_after["current_epoch"] == state_before["current_epoch"]
        assert ol_summary_after["expected_block_count"] == ol_summary_before["expected_block_count"]
        assert (
            checkpoints_after["checkpoints_found_in_db"]
            == checkpoints_before["checkpoints_found_in_db"]
        )
        checkpoint_after = run_dbtool_json(datadir, "get-checkpoint", str(latest_epoch))
        assert checkpoint_after.get("checkpoint_epoch") is not None
        # blocks should remain readable after dry run
        run_dbtool_json(datadir, "get-ol-state", target_block_id)
        run_dbtool_json(datadir, "get-ol-block", tip_id)
        logger.info("no state/block/checkpoint mutations in dry run")
        return True
