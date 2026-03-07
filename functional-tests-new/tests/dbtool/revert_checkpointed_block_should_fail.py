"""Revert inside checkpointed epoch should fail without -c."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import (
    get_latest_checkpoint,
    revert_ol_state,
    setup_revert_ol_state_test,
    target_start_of_checkpointed_epoch,
)

logger = logging.getLogger(__name__)


@flexitest.register
class RevertCheckpointedBlockShouldFailTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        logger.info("Starting checkpointed-block revert failure test (without -c)")
        seq_service = self.get_service(ServiceType.Strata)
        btc_service = self.get_service(ServiceType.Bitcoin)
        setup = setup_revert_ol_state_test(seq_service, btc_service)
        logger.info("Waiting for chain activity and confirmed progress")
        seq_service.stop()

        datadir = setup["datadir"]
        logger.info("Reading checkpoint summary from datadir=%s", datadir)
        latest_checkpoint = get_latest_checkpoint(datadir)
        latest_epoch = latest_checkpoint["latest_epoch"]
        target_block_id, _ = target_start_of_checkpointed_epoch(
            datadir,
            latest_checkpoint["checkpoint"],
            latest_checkpoint["epoch_summary"],
        )
        logger.info(
            "Targeting checkpointed epoch start: epoch=%s block_id=%s",
            latest_epoch,
            target_block_id,
        )

        code, stdout, stderr = revert_ol_state(datadir, target_block_id)
        assert code != 0
        logger.info("revert without -c failed for checkpointed target")
        return True
