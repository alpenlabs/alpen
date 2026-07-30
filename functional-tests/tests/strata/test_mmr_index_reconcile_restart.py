"""Strata restart repairs OL-owned MMR index entries after a crash."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, GenesisAccountData, ServiceType
from common.config.constants import ALPEN_ACCOUNT_ID
from common.crash_helpers import crash_and_recover
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import (
    get_mmr_leaf_count,
    run_dbtool_json,
    submit_generic_account_message,
)

logger = logging.getLogger(__name__)

CHAIN_WORKER_AFTER_MMR_INDEX_BAIL = "chain_worker_after_mmr_index"
INBOX_MMR_ID = f"snark-msg-inbox:{ALPEN_ACCOUNT_ID}"
L1_BLOCK_REFS_MMR_ID = "l1-block-refs"


@flexitest.register
class TestMmrIndexReconcileRestart(StrataNodeTest):
    """Restarts a sequencer after MMR indexing persists ahead of OL state."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                genesis_accounts={ALPEN_ACCOUNT_ID: GenesisAccountData()},
                epoch_sealing=EpochSealingConfig.new_fixed_slot(4),
                ol_block_time_ms=10_000,
            )
        )

    def main(self, ctx):
        strata = self.get_service(ServiceType.Strata)
        signer = self.get_service(ServiceType.StrataSigner)
        datadir = strata.props["datadir"]
        rpc = strata.wait_for_rpc_ready(timeout=20)
        submit_rpc = strata.create_submit_rpc()

        # Start near a fresh block boundary so the submit below has time to
        # reach the mempool before the next block trips the armed bail point.
        strata.wait_for_additional_blocks(2, rpc, timeout_per_block=15)
        target_status = strata.get_sync_status(rpc)
        target_tip = target_status["tip"]
        target_slot = int(target_tip["slot"])
        target_block_id = target_tip["blkid"]
        target_inbox_count = self._block_summary_inbox_count(rpc, target_slot)
        logger.info(
            "Selected pre-crash OL target: slot=%s block_id=%s inbox_count=%s",
            target_slot,
            target_block_id,
            target_inbox_count,
        )

        def submit_message_after_arm() -> None:
            txid = submit_generic_account_message(
                submit_rpc,
                ALPEN_ACCOUNT_ID,
                b"startup-mmr-index-repair",
            )
            logger.info("Submitted inbox GAM txid=%s after arming crash bail", txid)

        def inspect_crashed_mmr_index(pre_status: dict) -> None:
            pre_tip = pre_status["tip"]
            if pre_tip["blkid"] != target_block_id:
                raise AssertionError(
                    "chain tip advanced before arming MMR-index crash bail: "
                    f"expected {target_block_id}, got {pre_tip['blkid']}"
                )

            inbox_count = get_mmr_leaf_count(datadir, INBOX_MMR_ID)
            assert inbox_count > target_inbox_count, (
                f"expected crashed MMR index to be ahead of target inbox count "
                f"({inbox_count} <= {target_inbox_count})"
            )
            logger.info(
                "Crashed MMR %s is ahead of target: %s > %s",
                INBOX_MMR_ID,
                inbox_count,
                target_inbox_count,
            )

        result = crash_and_recover(
            strata,
            CHAIN_WORKER_AFTER_MMR_INDEX_BAIL,
            expected_block_advance=2,
            after_arm=submit_message_after_arm,
            after_crash=inspect_crashed_mmr_index,
            restart_timeout=30,
            recovery_timeout=90,
        )

        rpc = strata.create_rpc()
        post_tip = result.post_status["tip"]
        post_slot = int(post_tip["slot"])
        post_block_id = post_tip["blkid"]
        post_inbox_count = self._block_summary_inbox_count(rpc, post_slot)

        signer.stop()
        strata.stop()

        post_state = run_dbtool_json(datadir, "get-ol-state", post_block_id)
        post_l1_count = int(post_state["l1_next_expected_height"])
        assert get_mmr_leaf_count(datadir, INBOX_MMR_ID) == post_inbox_count
        assert get_mmr_leaf_count(datadir, L1_BLOCK_REFS_MMR_ID) == post_l1_count

        logger.info(
            "MMR index is consistent after restart: slot=%s inbox_count=%s l1_count=%s",
            post_slot,
            post_inbox_count,
            post_l1_count,
        )
        return True

    @staticmethod
    def _block_summary_inbox_count(rpc, slot: int) -> int:
        summaries = rpc.strata_getBlocksSummaries(ALPEN_ACCOUNT_ID, slot, slot)
        if not summaries:
            raise AssertionError(f"Alpen snark account block summary not found at slot {slot}")
        return int(summaries[-1]["next_inbox_msg_idx"])
