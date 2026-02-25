"""Parity test for legacy validate_syncinfo using OL commands."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import (
    load_rollup_genesis_height,
    ol_genesis_slot,
    run_dbtool_json,
    setup_revert_ol_state_test,
)

logger = logging.getLogger(__name__)


@flexitest.register
class DbtoolValidateSyncinfoTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(StrataEnvConfig(pre_generate_blocks=110, epoch_slots=4))

    def main(self, ctx):
        seq_service = self.get_service(ServiceType.Strata)
        btc_service = self.get_service(ServiceType.Bitcoin)
        seq_rpc = seq_service.wait_for_rpc_ready(timeout=20)
        initial_slot = seq_service.get_cur_block_height(seq_rpc)
        setup_revert_ol_state_test(seq_service, btc_service)

        seq_service.stop()
        datadir = seq_service.props["datadir"]
        genesis_height = load_rollup_genesis_height(datadir)

        logger.info("Testing get-syncinfo to validate chain positions")
        syncinfo = run_dbtool_json(datadir, "get-syncinfo")
        l1_tip_height = syncinfo.get("l1_tip_height", 0)
        ol_tip_height = syncinfo.get("ol_tip_height", 0)
        assert l1_tip_height > 0
        assert ol_tip_height >= initial_slot
        logger.info(
            "sync positions are valid (l1_tip=%s ol_tip=%s)",
            l1_tip_height,
            ol_tip_height,
        )

        logger.info("Testing get-l1-summary to verify L1 blocks exist")
        l1_summary = run_dbtool_json(datadir, "get-l1-summary", str(genesis_height))
        assert l1_summary.get("expected_block_count", 0) > 0
        assert l1_summary.get("all_manifests_present", False) is True
        logger.info("L1 summary shows expected blocks/manifests")

        logger.info("Testing get-ol-summary to verify OL blocks exist")
        ol_summary = run_dbtool_json(datadir, "get-ol-summary", str(ol_genesis_slot()))
        assert ol_summary.get("tip_slot", 0) > 0
        assert ol_summary.get("all_blocks_present", False) is True
        logger.info("OL summary shows expected blocks")

        logger.info("Testing get-checkpoints-summary to verify checkpoints exist")
        checkpoints = run_dbtool_json(datadir, "get-checkpoints-summary", str(genesis_height))
        assert checkpoints["checkpoints_found_in_db"] >= checkpoints["expected_checkpoints_count"]
        logger.info("checkpoints summary is consistent")
        return True
