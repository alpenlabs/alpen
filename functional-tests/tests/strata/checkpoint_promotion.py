"""Shared setup and assertions for checkpoint-datadir promotion tests."""

import copy
import logging
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

from common.accounts import RECIPIENT_ADDRESS, get_dev_account
from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.evm_utils import send_raw_transaction, wait_for_receipt
from common.rpc_types.strata import AccountEpochSummary, EpochCommitment, OLBlockInfo
from common.services.alpen_client import AlpenClientService
from common.services.signer import SignerService
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from tests.checkpoint.helpers import (
    mine_until_finalized_epoch,
    parse_checkpoint_epoch,
    wait_for_checkpoint_duty,
)

logger = logging.getLogger(__name__)

MIN_FINALIZED_EPOCH = 2
ZERO_HASH = "00" * 32


@dataclass
class FinalizedAnchor:
    """Checkpoint-sync state captured before the original sequencer is destroyed."""

    commitment: EpochCommitment
    summaries: dict[int, AccountEpochSummary]
    active_epoch: int

    @property
    def epoch(self) -> int:
        return self.commitment["epoch"]

    @property
    def slot(self) -> int:
        return self.commitment["last_slot"]

    @property
    def blkid(self) -> str:
        return self.commitment["last_blkid"]


@dataclass
class PromotedChain:
    """Promoted service state plus the dead sequencer fork recorded before deletion."""

    service: StrataService
    signer: SignerService
    rpc: Any
    old_tip: OLBlockInfo
    old_blocks: dict[int, dict[str, Any]]


def finalize_active_checkpoint(test: BaseTest) -> FinalizedAnchor:
    """Drives EE activity, then finalizes and records a checkpoint-sync anchor."""
    logger.info("driving EE/account activity before recovery")
    sequencer = test.get_service(ServiceType.Strata)
    checkpoint_node = cast(StrataService, test.get_service(ServiceType.StrataCheckpointNode))
    bitcoin = test.get_service(ServiceType.Bitcoin)
    alpen = cast(AlpenClientService, test.get_service(ServiceType.AlpenSequencer))

    sequencer_rpc = sequencer.wait_for_rpc_ready(timeout=30)
    checkpoint_rpc = checkpoint_node.wait_for_rpc_ready(timeout=30)
    alpen.wait_for_ready(timeout=60)
    alpen_rpc = alpen.create_rpc()
    btc_rpc = bitcoin.create_rpc()

    account = get_dev_account(alpen_rpc)
    gas_price = int(alpen_rpc.eth_gasPrice(), 16)
    raw_tx = account.sign_transfer(
        to=RECIPIENT_ADDRESS,
        value=1_000_000,
        gas_price=gas_price,
        gas=21_000,
    )
    tx_hash = send_raw_transaction(alpen_rpc, raw_tx)
    receipt = wait_for_receipt(alpen_rpc, tx_hash, timeout=120)
    assert receipt["status"] == "0x1", f"EE activity transaction failed: {receipt}"
    receipt_block = int(receipt["blockNumber"], 16)
    alpen.wait_for_block(receipt_block + 4, timeout=120)
    logger.info("EE transfer %s included at block %s", tx_hash, receipt_block)

    active_epoch = _wait_for_active_epoch(sequencer, sequencer_rpc, btc_rpc)
    target_epoch = max(MIN_FINALIZED_EPOCH, active_epoch)

    logger.info("finalizing checkpoint-sync node through epoch %s", target_epoch)
    finalized = mine_until_finalized_epoch(
        bitcoin=bitcoin,
        strata=checkpoint_node,
        strata_rpc=checkpoint_rpc,
        target_epoch=target_epoch,
        timeout=180,
        step=0.5,
    )
    commitment = cast(EpochCommitment, finalized)
    assert commitment["epoch"] >= MIN_FINALIZED_EPOCH
    assert commitment["last_blkid"] != ZERO_HASH

    summaries = {
        epoch: checkpoint_node.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch, checkpoint_rpc)
        for epoch in range(1, commitment["epoch"] + 1)
    }
    assert_summaries_equivalent(
        sequencer.get_account_epoch_summary(ALPEN_ACCOUNT_ID, active_epoch, sequencer_rpc),
        summaries[active_epoch],
    )
    logger.info(
        "checkpoint-sync anchor epoch=%s slot=%s blkid=%s",
        commitment["epoch"],
        commitment["last_slot"],
        commitment["last_blkid"],
    )
    return FinalizedAnchor(commitment, summaries, active_epoch)


def nuke_sequencer_and_promote(test: BaseTest, anchor: FinalizedAnchor) -> PromotedChain:
    """Deletes the old sequencer datadir and starts the pre-provisioned promotion."""
    sequencer = test.get_service(ServiceType.Strata)
    old_signer = cast(SignerService, test.get_service(ServiceType.StrataSigner))
    checkpoint_node = cast(StrataService, test.get_service(ServiceType.StrataCheckpointNode))
    promoted = cast(StrataService, test.get_service(ServiceType.StrataPromotedSequencer))
    promoted_signer = cast(SignerService, test.get_service(ServiceType.StrataPromotedSigner))

    sequencer_rpc = sequencer.create_rpc()
    old_tip = wait_until_with_value(
        lambda: sequencer.get_sync_status(sequencer_rpc)["tip"],
        lambda tip: tip["slot"] > anchor.slot,
        error_with="old sequencer tip did not advance above the checkpoint anchor",
        timeout=60,
        step=0.2,
    )
    old_blocks: dict[int, dict[str, Any]] = {}
    for slot in range(anchor.slot + 1, old_tip["slot"] + 1):
        block = sequencer_rpc.strata_getBlockBySlot(slot)
        assert block is not None, f"old sequencer block missing at unconfirmed slot {slot}"
        old_blocks[slot] = block

    logger.info(
        "stopping old sequencer at slot %s and preserving %s fork blocks",
        old_tip["slot"],
        len(old_blocks),
    )
    old_signer.stop()
    sequencer.stop()

    source_key_value = sequencer.props["sequencer_key_path"]
    assert source_key_value is not None, "original sequencer key path was not exposed"
    source_key = Path(source_key_value)
    copied_key = Path(promoted_signer.props["sequencer_key_path"])
    assert source_key.is_file(), f"sequencer key does not exist: {source_key}"
    assert Path(sequencer.props["datadir"]) not in copied_key.parents
    shutil.copy2(source_key, copied_key)
    assert copied_key.read_bytes() == source_key.read_bytes()

    old_datadir = Path(sequencer.props["datadir"])
    shutil.rmtree(old_datadir)
    assert not old_datadir.exists(), f"old sequencer datadir still exists: {old_datadir}"

    logger.info("promoting checkpoint-sync datadir %s", promoted.props["datadir"])
    checkpoint_node.stop()
    assert "--bootstrap-from-checkpoint" in promoted.cmd
    promoted.start()
    promoted_rpc = promoted.wait_for_rpc_ready(timeout=60)

    status = promoted.get_sync_status(promoted_rpc)
    assert status["finalized"] == anchor.commitment, status
    assert status["confirmed"] == anchor.commitment, status
    assert status["latest"] == anchor.commitment, status
    assert status["tip"]["epoch"] == anchor.epoch, status
    assert status["tip"]["slot"] == anchor.slot, status
    assert status["tip"]["blkid"] == anchor.blkid, status

    promoted_signer.start()
    promoted_signer.wait_for_ready(timeout=10)
    logger.info("promoted node started exactly at the finalized anchor")
    return PromotedChain(promoted, promoted_signer, promoted_rpc, old_tip, old_blocks)


def assert_sequencing_resumed(anchor: FinalizedAnchor, promoted: PromotedChain) -> None:
    """Checks the header-only parent and that recorded dead-fork slots are re-authored."""
    logger.info("waiting for sequencing to resume after slot %s", anchor.slot)
    promoted.service.wait_for_block_height(
        anchor.slot + 1,
        promoted.rpc,
        timeout=60,
        poll_interval=0.2,
    )
    first_block = promoted.rpc.strata_getBlockBySlot(anchor.slot + 1)
    assert first_block is not None
    assert first_block["header"]["parent_blkid"] == anchor.blkid, first_block

    if promoted.old_blocks:
        promoted.service.wait_for_block_height(
            max(promoted.old_blocks),
            promoted.rpc,
            timeout=60,
            poll_interval=0.2,
        )
    for slot, old_block in promoted.old_blocks.items():
        new_block = promoted.rpc.strata_getBlockBySlot(slot)
        assert new_block is not None, f"promoted block missing at re-authored slot {slot}"
        assert new_block["header"]["blkid"] != old_block["header"]["blkid"], (
            f"slot {slot} retained dead-fork blkid {old_block['header']['blkid']}"
        )
    logger.info(
        "sequencing resumes: header-only parent used and %s dead-fork slots re-authored",
        len(promoted.old_blocks),
    )


def finalize_promoted_epoch(
    test: BaseTest,
    anchor: FinalizedAnchor,
    promoted: PromotedChain,
    target_epoch: int | None = None,
) -> int:
    """Waits for a promoted checkpoint duty and proves its L1 finalization."""
    bitcoin = test.get_service(ServiceType.Bitcoin)
    btc_rpc = bitcoin.create_rpc()
    if target_epoch is None:
        target_epoch = anchor.epoch + 1

    logger.info(
        "checkpoint published: waiting for promoted checkpoint duty at epoch %s", target_epoch
    )
    duty = wait_for_checkpoint_duty(
        promoted.service.create_admin_rpc(),
        timeout=120,
        step=0.1,
        min_epoch=target_epoch,
    )
    duty_epoch = parse_checkpoint_epoch(duty)
    # Duties surface the earliest unsigned checkpoint, and the attached signer
    # completes them concurrently. Observing a later epoch's duty means the
    # target epoch was already signed, which is the success condition; the
    # finalization assertions below still pin the target epoch end to end.
    assert duty_epoch >= target_epoch, (
        f"expected promoted checkpoint duty at or past epoch {target_epoch}, got {duty_epoch}"
    )

    mine_until_finalized_epoch(
        bitcoin=bitcoin,
        strata=promoted.service,
        strata_rpc=promoted.rpc,
        target_epoch=target_epoch,
        timeout=180,
        step=0.5,
    )
    info = promoted.rpc.strata_getCheckpointInfo(target_epoch)
    assert info is not None, f"missing checkpoint info for promoted epoch {target_epoch}"
    confirmation = info["confirmation_status"]
    assert confirmation["status"] == "finalized", confirmation
    l1_reference = confirmation["l1_reference"]
    assert l1_reference["txid"] != ZERO_HASH, l1_reference
    tx = btc_rpc.proxy.getrawtransaction(l1_reference["txid"], 1)
    assert tx["txid"] == l1_reference["txid"]
    logger.info(
        "checkpoint published: promoted epoch %s finalized in tx %s",
        target_epoch,
        l1_reference["txid"],
    )
    return target_epoch


def assert_fresh_checkpoint_recovery(
    test: BaseTest,
    promoted: PromotedChain,
    target_epoch: int,
) -> None:
    """Starts a fresh L1-only checkpoint sync node and compares the promoted epoch summary."""
    logger.info("DA continuity: starting fresh checkpoint-sync node from an empty datadir")
    bitcoin = test.get_service(ServiceType.Bitcoin)
    fresh = cast(StrataService, test.get_service(ServiceType.StrataRecoveryCheckpointNode))
    fresh.start()
    fresh_rpc = fresh.wait_for_rpc_ready(timeout=60)
    finalized = mine_until_finalized_epoch(
        bitcoin=bitcoin,
        strata=fresh,
        strata_rpc=fresh_rpc,
        target_epoch=target_epoch,
        timeout=180,
        step=0.5,
    )
    assert finalized["epoch"] >= target_epoch

    promoted_summary = promoted.service.get_account_epoch_summary(
        ALPEN_ACCOUNT_ID, target_epoch, promoted.rpc
    )
    fresh_summary = fresh.get_account_epoch_summary(ALPEN_ACCOUNT_ID, target_epoch, fresh_rpc)
    assert_summaries_equivalent(promoted_summary, fresh_summary)
    logger.info("DA continuity: fresh node reconstructed promoted epoch %s from L1", target_epoch)


def assert_summaries_equivalent(
    expected: AccountEpochSummary,
    actual: AccountEpochSummary,
) -> None:
    """Compares checkpoint summaries while allowing omitted intermediate state roots."""
    expected_copy = copy.deepcopy(dict(expected))
    actual_copy = copy.deepcopy(dict(actual))
    expected_updates = cast(list[dict[str, Any]], expected_copy.pop("update_inputs"))
    actual_updates = cast(list[dict[str, Any]], actual_copy.pop("update_inputs"))
    assert expected_copy == actual_copy

    for expected_update, actual_update in zip(expected_updates, actual_updates, strict=True):
        expected_root = expected_update.pop("new_state_root")
        actual_root = actual_update.pop("new_state_root")
        assert actual_root is None or actual_root == expected_root
        assert actual_update == expected_update


def _wait_for_active_epoch(sequencer: StrataService, sequencer_rpc, btc_rpc) -> int:
    next_epoch = 1
    for _ in range(30):
        status = wait_until_with_value(
            lambda: _mine_and_get_status(sequencer, sequencer_rpc, btc_rpc),
            lambda value, epoch=next_epoch: value["tip"]["epoch"] > epoch,
            error_with=f"sequencer did not complete epoch {next_epoch}",
            timeout=120,
            step=0.5,
        )
        for epoch in range(next_epoch, status["tip"]["epoch"]):
            summary = sequencer.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch, sequencer_rpc)
            if summary["update_inputs"]:
                logger.info("account activity landed in OL epoch %s", epoch)
                return epoch
        next_epoch = status["tip"]["epoch"]
    raise AssertionError("no Alpen account activity found within 30 completed OL epochs")


def _mine_and_get_status(sequencer: StrataService, sequencer_rpc, btc_rpc):
    btc_rpc.proxy.generatetoaddress(2, btc_rpc.proxy.getnewaddress())
    return sequencer.get_sync_status(sequencer_rpc)
