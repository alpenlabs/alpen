"""Revert to finalized block range should fail."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import (
    parse_finalized_epoch_from_syncinfo,
    parse_ol_block_parent_blkid,
    parse_ol_block_slot,
    revert_ol_state,
    run_dbtool_json,
    setup_revert_ol_state_test,
)

logger = logging.getLogger(__name__)


@flexitest.register
class RevertFinalizedBlockShouldFailTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        logger.info("Starting finalized-block revert failure test")
        seq_service = self.get_service(ServiceType.Strata)
        btc_service = self.get_service(ServiceType.Bitcoin)
        setup_revert_ol_state_test(seq_service, btc_service)
        seq_service.stop()

        datadir = seq_service.props["datadir"]
        syncinfo = run_dbtool_json(datadir, "get-syncinfo")
        finalized_block, finalized_slot = parse_finalized_epoch_from_syncinfo(syncinfo)
        finalized_block_data = run_dbtool_json(datadir, "get-ol-block", finalized_block)
        target_block_id = parse_ol_block_parent_blkid(finalized_block_data)
        target_slot = parse_ol_block_slot(finalized_block_data) - 1
        assert target_slot == finalized_slot - 1
        logger.info("Finalized epoch last slot: %s", finalized_slot)
        logger.info("Targeting slot %s", target_slot)
        logger.info("Attempting revert to finalized parent block_id=%s", target_block_id)

        code, stdout, stderr = revert_ol_state(datadir, target_block_id)
        assert code != 0
        logger.info("revert failed as expected for finalized target")
        return True
