"""Sequencer revert-ol-state should pop OL-owned MMR index namespaces.

The test submits a direct GenericAccountMessage to a genesis snark account.
OL processing appends that message to the account's inbox MMR, and the chain
worker mirrors the append into MmrIndexDb. It also mines post-target L1 blocks
so `l1-block-refs` advances past the revert target.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, GenesisAccountData, ServiceType
from common.config.constants import ALPEN_ACCOUNT_ID
from common.wait import wait_until_with_value
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import (
    get_latest_checkpoint,
    get_mmr_leaf_count,
    restart_sequencer_after_revert,
    revert_ol_state,
    run_dbtool_json,
    setup_revert_ol_state_test,
    submit_generic_account_message,
    target_end_of_checkpointed_epoch,
    verify_tip_resumed_with_new_blkid,
)

logger = logging.getLogger(__name__)

INBOX_MMR_ID = f"snark-msg-inbox:{ALPEN_ACCOUNT_ID}"
L1_BLOCK_REFS_MMR_ID = "l1-block-refs"
EXTRA_L1_BLOCKS = 2


@flexitest.register
class RevertOLStateMmrIndexTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                genesis_accounts={ALPEN_ACCOUNT_ID: GenesisAccountData()},
                epoch_sealing=EpochSealingConfig.new_fixed_slot(4),
            )
        )

    def main(self, ctx):
        seq_service = self.get_service(ServiceType.Strata)
        signer_service = self.get_service(ServiceType.StrataSigner)
        btc_service = self.get_service(ServiceType.Bitcoin)
        setup = setup_revert_ol_state_test(seq_service, btc_service)
        seq_rpc = setup["rpc"]
        submit_rpc = seq_service.create_submit_rpc()
        btc_rpc = btc_service.create_rpc()
        slots_per_epoch = seq_service.props.get("slots_per_epoch")
        if not isinstance(slots_per_epoch, int) or slots_per_epoch <= 0:
            raise AssertionError(f"Invalid slots_per_epoch in sequencer props: {slots_per_epoch!r}")

        target_sync = seq_service.get_sync_status(seq_rpc)
        target_block_id = target_sync["tip"]["blkid"]
        target_slot = int(target_sync["tip"]["slot"])
        target_inbox_count = self._block_summary_inbox_count(seq_rpc, target_slot)
        logger.info(
            "Selected pre-message revert target: slot=%s block_id=%s inbox_count=%s",
            target_slot,
            target_block_id,
            target_inbox_count,
        )

        pre_l1_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        mine_addr = btc_rpc.proxy.getnewaddress()
        for _ in range(EXTRA_L1_BLOCKS):
            btc_rpc.proxy.generatetoaddress(1, mine_addr)
        post_l1_tip = btc_rpc.proxy.getblockchaininfo()["blocks"]
        expected_post_l1_tip = pre_l1_tip + EXTRA_L1_BLOCKS
        if post_l1_tip != expected_post_l1_tip:
            raise AssertionError(f"Expected L1 tip {expected_post_l1_tip}, got {post_l1_tip}")
        seq_service.wait_for_asm_manifest_commitment_at(post_l1_tip, rpc=seq_rpc, timeout=120)
        logger.info("Strata observed post-target L1 tip %s", post_l1_tip)

        txid = submit_generic_account_message(
            submit_rpc,
            ALPEN_ACCOUNT_ID,
            b"dbtool-mmr-inbox-revert",
        )
        logger.info("Submitted inbox GAM txid=%s", txid)

        post_message_summary = self._wait_for_inbox_append_after(
            seq_service,
            seq_rpc,
            target_slot + 1,
            target_inbox_count,
        )
        post_message_inbox_count = int(post_message_summary["next_inbox_msg_idx"])
        if post_message_inbox_count <= target_inbox_count:
            raise AssertionError(
                f"inbox count did not advance: before={target_inbox_count} "
                f"after={post_message_inbox_count}"
            )

        seq_service.wait_for_additional_blocks(
            slots_per_epoch,
            seq_rpc,
            timeout_per_block=10,
        )

        live_sync = seq_service.get_sync_status(seq_rpc)
        old_live_tip_slot = int(live_sync["tip"]["slot"])
        old_live_tip_blkid = live_sync["tip"]["blkid"]
        logger.info(
            "Pre-revert live tip: slot=%s blkid=%s inbox_count=%s",
            old_live_tip_slot,
            old_live_tip_blkid,
            post_message_inbox_count,
        )

        signer_service.stop()
        seq_service.stop()

        datadir = seq_service.props["datadir"]
        pre_revert_sync = run_dbtool_json(datadir, "get-syncinfo")
        pre_revert_tip_slot = int(pre_revert_sync["ol_tip_height"])
        expected_reverted_blocks = pre_revert_tip_slot - target_slot
        if expected_reverted_blocks <= 0:
            raise AssertionError(
                f"expected target slot {target_slot} to be below pre-revert tip "
                f"{pre_revert_tip_slot}"
            )

        latest_checkpoint = get_latest_checkpoint(datadir)
        _, latest_checkpoint_slot = target_end_of_checkpointed_epoch(latest_checkpoint)
        inside_checkpointed_epoch = target_slot < latest_checkpoint_slot
        logger.info(
            "Revert target checkpoint relation: target_slot=%s "
            "latest_checkpoint_slot=%s inside_checkpointed_epoch=%s",
            target_slot,
            latest_checkpoint_slot,
            inside_checkpointed_epoch,
        )

        target_state_before = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        assert int(target_state_before["current_slot"]) == target_slot
        # `get-ol-state` does not expose l1_block_refs_mmr.num_entries().
        # The regtest genesis sentinel plus one leaf per accepted L1 height
        # makes that target count equal next_expected_height in this environment.
        target_l1_count = int(target_state_before["l1_next_expected_height"])

        inbox_count_before_revert = get_mmr_leaf_count(datadir, INBOX_MMR_ID)
        l1_count_before_revert = get_mmr_leaf_count(datadir, L1_BLOCK_REFS_MMR_ID)
        logger.info(
            "MMR %s before revert: leaf_count=%s",
            INBOX_MMR_ID,
            inbox_count_before_revert,
        )
        logger.info(
            "MMR %s before revert: leaf_count=%s target_count=%s",
            L1_BLOCK_REFS_MMR_ID,
            l1_count_before_revert,
            target_l1_count,
        )
        assert inbox_count_before_revert == post_message_inbox_count
        assert inbox_count_before_revert > target_inbox_count
        assert l1_count_before_revert > target_l1_count

        code, stdout, stderr = revert_ol_state(
            datadir,
            target_block_id,
            force=True,
            revert_checkpointed=inside_checkpointed_epoch,
        )
        assert code == 0, stderr or stdout
        assert f"MMR revert: {INBOX_MMR_ID}" in stdout
        assert f"MMR revert: {L1_BLOCK_REFS_MMR_ID}" in stdout
        assert (
            self._summary_count(stdout, "OL states/write batches to delete")
            == expected_reverted_blocks
        )
        assert self._summary_count(stdout, "Blocks to mark unchecked") == expected_reverted_blocks
        assert self._summary_count(stdout, "Blocks to delete") == 0

        inbox_count_after_revert = get_mmr_leaf_count(datadir, INBOX_MMR_ID)
        l1_count_after_revert = get_mmr_leaf_count(datadir, L1_BLOCK_REFS_MMR_ID)
        logger.info(
            "MMR %s after revert: leaf_count=%s",
            INBOX_MMR_ID,
            inbox_count_after_revert,
        )
        logger.info(
            "MMR %s after revert: leaf_count=%s",
            L1_BLOCK_REFS_MMR_ID,
            l1_count_after_revert,
        )
        assert inbox_count_after_revert == target_inbox_count
        assert inbox_count_after_revert < inbox_count_before_revert
        assert l1_count_after_revert == target_l1_count
        assert l1_count_after_revert < l1_count_before_revert

        target_state_after = run_dbtool_json(datadir, "get-ol-state", target_block_id)
        assert int(target_state_after["current_slot"]) == target_slot
        post_revert_sync = run_dbtool_json(datadir, "get-syncinfo")
        assert int(post_revert_sync["ol_tip_height"]) == target_slot

        seq_rpc, resumed_slot = restart_sequencer_after_revert(
            seq_service,
            old_live_tip_slot,
            signer_service=signer_service,
            error_with="Sequencer did not resume after MMR index revert",
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
        return True

    @staticmethod
    def _block_summary_inbox_count(seq_rpc, slot: int) -> int:
        summaries = seq_rpc.strata_getBlocksSummaries(ALPEN_ACCOUNT_ID, slot, slot)
        if not summaries:
            raise AssertionError(f"Alpen snark account block summary not found at slot {slot}")
        return int(summaries[-1]["next_inbox_msg_idx"])

    @staticmethod
    def _latest_inbox_summary_after(
        seq_service,
        seq_rpc,
        start_slot: int,
        min_inbox_count: int,
    ) -> dict | None:
        tip_slot = int(seq_service.get_sync_status(seq_rpc)["tip"]["slot"])
        if tip_slot < start_slot:
            return None

        summaries = seq_rpc.strata_getBlocksSummaries(ALPEN_ACCOUNT_ID, start_slot, tip_slot)
        advanced = [
            summary for summary in summaries if int(summary["next_inbox_msg_idx"]) > min_inbox_count
        ]
        return advanced[-1] if advanced else None

    @staticmethod
    def _wait_for_inbox_append_after(
        seq_service,
        seq_rpc,
        start_slot: int,
        min_inbox_count: int,
    ) -> dict:
        def latest_summary():
            return RevertOLStateMmrIndexTest._latest_inbox_summary_after(
                seq_service,
                seq_rpc,
                start_slot,
                min_inbox_count,
            )

        def has_summary(summary):
            return summary is not None

        return wait_until_with_value(
            latest_summary,
            has_summary,
            error_with="Snark account inbox did not append after GAM submission",
            timeout=60,
        )

    @staticmethod
    def _summary_count(stdout: str, label: str) -> int:
        prefix = f"{label}:"
        for line in stdout.splitlines():
            if line.startswith(prefix):
                return int(line.removeprefix(prefix).strip())
        raise AssertionError(f"Missing dbtool summary line: {label}")
