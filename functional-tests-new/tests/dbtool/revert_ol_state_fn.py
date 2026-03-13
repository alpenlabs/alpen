"""Test to revert OL state in fullnode.

NOTE: Disabled in entry.py because fullnode sync is not implemented in new `strata`.
"""

import logging

import flexitest

from common.base_test import BaseTest
from common.config import ServiceType
from envconfigs.strata_seq_fullnode import (
    STRATA_FULLNODE_SERVICE_NAME,
    StrataSequencerFullnodeEnvConfig,
)
from tests.dbtool.helpers import (
    get_latest_checkpoint,
    restart_fullnode_after_revert,
    revert_ol_state,
    run_dbtool_json,
    setup_revert_ol_state_test_fullnode,
    target_end_of_checkpointed_epoch,
    verify_checkpoint_preserved,
    wait_for_seq_fn_progress,
)

logger = logging.getLogger(__name__)


@flexitest.register
class RevertOLStateFnTest(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataSequencerFullnodeEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        seq_service = self.get_service(ServiceType.Strata)
        btc_service = self.get_service(ServiceType.Bitcoin)
        fn_service = self.get_service(STRATA_FULLNODE_SERVICE_NAME)
        setup = setup_revert_ol_state_test_fullnode(seq_service, fn_service, btc_service)
        old_seq_tip, old_fn_tip = wait_for_seq_fn_progress(
            seq_service,
            fn_service,
            setup["seq_rpc"],
            setup["fn_rpc"],
        )
        logger.info("Pre-revert tips: seq=%s fn=%s", old_seq_tip, old_fn_tip)

        seq_service.stop()
        fn_service.stop()
        datadir = fn_service.props["datadir"]
        latest_checkpoint = get_latest_checkpoint(datadir)
        latest_epoch_before_revert = int(latest_checkpoint["checkpoint_epoch"])
        post_restart_target_epoch = latest_epoch_before_revert + 1
        target_block_id, target_slot = target_end_of_checkpointed_epoch(latest_checkpoint)
        logger.info(
            "Target slot: %s, target block ID: %s (end of checkpointed epoch)",
            target_slot,
            target_block_id,
        )
        logger.info(
            "Testing revert-ol-state to %s using fullnode database",
            target_block_id,
        )

        code, stdout, stderr = revert_ol_state(datadir, target_block_id)
        assert code == 0, stderr or stdout

        target_after = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        assert int(target_after["current_slot"]) == target_slot
        logger.info("Revert verification passed")
        assert verify_checkpoint_preserved(datadir, latest_epoch_before_revert)
        logger.info("Checkpoint and epoch summary are preserved")

        # Restart both services and verify resync/progression.
        _, _, new_seq_tip, new_fn_tip = restart_fullnode_after_revert(
            seq_service,
            fn_service,
            old_seq_tip,
            old_fn_tip,
            target_epoch=post_restart_target_epoch,
        )
        assert new_seq_tip > old_seq_tip
        assert new_fn_tip > old_fn_tip
        logger.info(
            "Criterion passed: fullnode observed checkpoint creation for "
            "latest checkpoint epoch before revert + 1 "
            "(latest_before_revert=%s target=%s)",
            latest_epoch_before_revert,
            post_restart_target_epoch,
        )
        logger.info("After restart - Sequencer OL: %s", new_seq_tip)
        logger.info("After restart - Fullnode OL: %s", new_fn_tip)
        logger.info("Sequencer and fullnode resumed after revert")
        return True
