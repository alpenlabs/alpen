"""Fullnode checkpointed-block revert with -c should succeed."""

import logging
import time

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
    target_start_of_checkpointed_epoch,
    verify_checkpoint_deleted,
    wait_for_finalized_epoch_with_mining,
    wait_for_seq_fn_progress,
)

logger = logging.getLogger(__name__)


@flexitest.register
class RevertCheckpointedBlockFnTest(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataSequencerFullnodeEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        logger.info("Starting fullnode checkpointed-block revert test (-c)")
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
        time.sleep(2)

        seq_service.stop()
        fn_service.stop()
        datadir = setup["fn_datadir"]
        latest_checkpoint = get_latest_checkpoint(datadir)
        latest_epoch_before_revert = latest_checkpoint["latest_epoch"]
        target_finalized_epoch = latest_epoch_before_revert + 1
        epoch_summary_before = run_dbtool_json(
            datadir, "get-epoch-summary", str(latest_epoch_before_revert)
        )
        logger.info("Epoch summary before revert: %s", epoch_summary_before)
        target_block_id, target_slot = target_start_of_checkpointed_epoch(
            datadir,
            latest_checkpoint["checkpoint"],
            latest_checkpoint["epoch_summary"],
        )
        logger.info(
            "Executing fullnode revert -f -c to epoch-start block_id=%s",
            target_block_id,
        )

        code, stdout, stderr = revert_ol_state(
            datadir,
            target_block_id,
            revert_checkpointed=True,
        )
        assert code == 0, stderr or stdout

        target_after = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        assert int(target_after["current_slot"]) == target_slot
        logger.info("target OL state slot preserved after revert")
        # Reverting to the start of checkpointed epoch should delete that epoch's metadata.
        assert verify_checkpoint_deleted(datadir, latest_epoch_before_revert)
        logger.info("checkpoint and epoch summary deleted for reverted epoch")

        # Restart both services and verify fullnode catches up.
        seq_rpc, _, new_seq_tip, new_fn_tip = restart_fullnode_after_revert(
            seq_service,
            fn_service,
            old_seq_tip,
            old_fn_tip,
        )
        assert new_seq_tip > old_seq_tip
        assert new_fn_tip > old_fn_tip
        wait_for_finalized_epoch_with_mining(
            seq_service,
            seq_rpc,
            setup["btc_rpc"],
            setup["mine_address"],
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
        logger.info("seq resumed and fullnode progressed after revert")
        return True
