"""Functional test for EVM precompile-driven subject transfer across two EEs."""

import logging
from typing import cast

import flexitest
from eth_account import Account
from eth_utils import to_checksum_address

from common.base_test import BaseTest
from common.bridge import ee_log_path, rpc_quantity_to_int, wait_for_output_snark_update
from common.config.constants import ALPEN_ACCOUNT_ID, NEPAL_ACCOUNT_ID, SATS_TO_WEI, ServiceType
from common.evm_utils import subject_hex_from_address, wait_for_ee_balance
from common.ol_utils import (
    count_new_inbox_messages,
    get_ol_balance,
    wait_for_account_update_exact_seq,
    wait_for_inbox_message_delta,
    wait_for_next_ol_epoch,
    wait_for_ol_balance,
)
from common.precompile import PRECOMPILE_SUBJECT_TRANSFER_ADDRESS, wait_for_receipt
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.test_cli import create_mock_deposit
from envconfigs.el_ol import NEPAL_SEQUENCER_SERVICE

logger = logging.getLogger(__name__)

ALPEN_ACCOUNT_SERIAL = 128
TRANSFER_AMOUNT_SATS = 100_000_000
DEPOSIT_AMOUNT_SATS = 2 * TRANSFER_AMOUNT_SATS
TRANSFER_DATA_HEX = "707265636f6d70696c652d7375626a2d7472616e73666572"
SUBJECT_TRANSFER_MSG_TYPE_HEX = "01"
GAS_LIMIT = 200_000


def submit_subject_transfer_precompile(
    alpen_rpc,
    sender_address: str,
    sender_private_key_hex: str,
    dest_account_hex: str,
    dest_subject_hex: str,
    amount_sats: int,
    transfer_data_hex: str = "",
) -> str:
    """Submit a subject-transfer precompile transaction from an EE account."""
    chain_id = int(alpen_rpc.eth_chainId(), 16)
    gas_price = int(alpen_rpc.eth_gasPrice(), 16)
    nonce = int(alpen_rpc.eth_getTransactionCount(sender_address, "latest"), 16)
    calldata_hex = dest_account_hex + dest_subject_hex + transfer_data_hex

    tx = {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": GAS_LIMIT,
        "to": to_checksum_address(PRECOMPILE_SUBJECT_TRANSFER_ADDRESS),
        "value": amount_sats * SATS_TO_WEI,
        "data": bytes.fromhex(calldata_hex),
        "chainId": chain_id,
    }
    signed = Account.sign_transaction(tx, sender_private_key_hex)
    return alpen_rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())


def assert_subject_transfer_receipt(alpen_rpc, tx_hash: str, timeout: int = 30) -> int:
    """Wait for subject-transfer receipt and return gas spent in wei."""
    receipt = wait_for_receipt(alpen_rpc, tx_hash, timeout=timeout)
    if receipt["status"] not in (1, "0x1"):
        raise AssertionError(f"subject-transfer precompile reverted: {receipt}")
    if not receipt["logs"]:
        raise AssertionError("subject-transfer precompile did not emit an intent event")

    gas_used = rpc_quantity_to_int(receipt["gasUsed"])
    gas_price = int(alpen_rpc.eth_gasPrice(), 16)
    effective_gas_price = rpc_quantity_to_int(receipt.get("effectiveGasPrice", gas_price))
    return gas_used * effective_gas_price


def get_new_inbox_messages(rpc, account_id_hex: str, start_slot: int) -> list[dict]:
    tip_slot = rpc.strata_getChainStatus()["tip"]["slot"]
    summaries = rpc.strata_getBlocksSummaries(account_id_hex, start_slot, tip_slot)
    messages = []
    for summary in summaries:
        messages.extend(summary.get("new_inbox_messages") or [])
    return messages


def assert_subject_transfer_inbox_message(
    rpc,
    account_id_hex: str,
    start_slot: int,
    source_subject_hex: str,
    dest_subject_hex: str,
    transfer_amount_sats: int,
    transfer_data_hex: str,
) -> None:
    transfer_data = bytes.fromhex(transfer_data_hex)
    assert len(transfer_data) < 128, "test helper only handles one-byte varint lengths"
    expected_payload = (
        SUBJECT_TRANSFER_MSG_TYPE_HEX
        + source_subject_hex
        + dest_subject_hex
        + len(transfer_data).to_bytes(1, "big").hex()
        + transfer_data_hex
    ).lower()

    for message in get_new_inbox_messages(rpc, account_id_hex, start_slot):
        payload = message["payload"]
        if payload["value"] != transfer_amount_sats:
            continue
        if payload["data"].removeprefix("0x").lower() == expected_payload:
            return

    raise AssertionError(
        "nepal inbox did not contain expected subject-transfer payload: "
        f"value={transfer_amount_sats}, data={expected_payload}"
    )


@flexitest.register
class TestPrecompileSubjectTransferTwoEEs(BaseTest):
    """EVM subject-transfer precompile creates an OL message consumed by another EE."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol_two_ees")

    def main(self, ctx) -> bool:
        del ctx
        strata = cast(StrataService, self.get_service(ServiceType.Strata))
        bitcoin = cast(BitcoinService, self.get_service(ServiceType.Bitcoin))
        alpen = cast(AlpenClientService, self.get_service(ServiceType.AlpenSequencer))
        nepal = cast(AlpenClientService, self.get_service(NEPAL_SEQUENCER_SERVICE))

        strata_rpc = strata.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()
        alpen_rpc = alpen.create_rpc()
        nepal_rpc = nepal.create_rpc()

        strata.wait_for_account_genesis_epoch_commitment(ALPEN_ACCOUNT_ID, strata_rpc, timeout=30)
        strata.wait_for_account_genesis_epoch_commitment(NEPAL_ACCOUNT_ID, strata_rpc, timeout=30)

        alpen_sender = Account.create()
        nepal_recipient = Account.create()
        alpen_subject = subject_hex_from_address(alpen_sender.address)
        nepal_subject = subject_hex_from_address(nepal_recipient.address)

        miner_addr = btc_rpc.proxy.getnewaddress()
        expected_deposit_wei = DEPOSIT_AMOUNT_SATS * SATS_TO_WEI

        create_mock_deposit(
            account_serial=ALPEN_ACCOUNT_SERIAL,
            amount=DEPOSIT_AMOUNT_SATS,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
            subject=alpen_subject,
        )
        btc_rpc.proxy.generatetoaddress(8, miner_addr)
        strata.wait_for_additional_blocks(
            2 * strata.props["slots_per_epoch"], strata_rpc, timeout_per_block=15
        )

        wait_for_ol_balance(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            DEPOSIT_AMOUNT_SATS,
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
        )
        wait_for_ee_balance(
            alpen_rpc,
            btc_rpc,
            miner_addr,
            alpen_sender.address,
            expected_deposit_wei,
        )

        alpen_balance_before_transfer = get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID)
        nepal_balance_before_transfer = get_ol_balance(strata_rpc, NEPAL_ACCOUNT_ID)
        assert nepal_balance_before_transfer == 0, (
            f"expected nepal OL balance to start at 0, got {nepal_balance_before_transfer}"
        )
        transfer_start_slot = strata_rpc.strata_getChainStatus()["tip"]["slot"]
        transfer_start_count = count_new_inbox_messages(
            strata_rpc, NEPAL_ACCOUNT_ID, transfer_start_slot
        )

        alpen_log = ee_log_path(alpen)
        alpen_output_log_offset = alpen_log.stat().st_size if alpen_log.exists() else 0
        start_epoch = wait_for_next_ol_epoch(strata_rpc, btc_rpc, miner_addr)

        logger.info("Calling Alpen subject-transfer precompile with Nepal destination account")
        tx_hash = submit_subject_transfer_precompile(
            alpen_rpc,
            alpen_sender.address,
            alpen_sender.key.hex(),
            NEPAL_ACCOUNT_ID,
            nepal_subject,
            TRANSFER_AMOUNT_SATS,
            TRANSFER_DATA_HEX,
        )
        gas_spent_wei = assert_subject_transfer_receipt(alpen_rpc, tx_hash)

        expected_alpen_sender_wei = (
            expected_deposit_wei - TRANSFER_AMOUNT_SATS * SATS_TO_WEI - gas_spent_wei
        )
        expected_alpen_sender_remainder_wei = TRANSFER_AMOUNT_SATS * SATS_TO_WEI - gas_spent_wei
        assert expected_alpen_sender_wei == expected_alpen_sender_remainder_wei, (
            "unexpected sender EE remainder calculation: "
            f"got {expected_alpen_sender_wei}, expected {expected_alpen_sender_remainder_wei}"
        )
        wait_for_ee_balance(
            alpen_rpc,
            btc_rpc,
            miner_addr,
            alpen_sender.address,
            expected_alpen_sender_wei,
        )

        alpen_seq_no = wait_for_output_snark_update(
            alpen_log,
            btc_rpc,
            miner_addr,
            after_offset=alpen_output_log_offset,
        )
        wait_for_account_update_exact_seq(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            alpen_seq_no,
            start_epoch,
            btc_rpc,
            miner_addr,
        )

        expected_alpen_ol_remainder = alpen_balance_before_transfer - TRANSFER_AMOUNT_SATS
        wait_for_ol_balance(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            expected_alpen_ol_remainder,
            timeout=120,
        )
        assert expected_alpen_ol_remainder == TRANSFER_AMOUNT_SATS, (
            "unexpected sender OL remainder calculation: "
            f"got {expected_alpen_ol_remainder}, expected {TRANSFER_AMOUNT_SATS}"
        )
        wait_for_ol_balance(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            nepal_balance_before_transfer + TRANSFER_AMOUNT_SATS,
            timeout=120,
        )
        wait_for_inbox_message_delta(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            transfer_start_slot,
            transfer_start_count,
            1,
            "nepal did not receive precompile-driven subject-transfer inbox message",
        )
        assert_subject_transfer_inbox_message(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            transfer_start_slot,
            alpen_subject,
            nepal_subject,
            TRANSFER_AMOUNT_SATS,
            TRANSFER_DATA_HEX,
        )
        wait_for_ee_balance(
            nepal_rpc,
            btc_rpc,
            miner_addr,
            nepal_recipient.address,
            TRANSFER_AMOUNT_SATS * SATS_TO_WEI,
        )

        logger.info("precompile-driven subject transfer reached OL and minted on Nepal")
        return True
