"""Functional test for two Alpen EEs sharing one Strata OL."""

import logging
from typing import cast

import flexitest
from eth_account import Account

from common.base_test import BaseTest
from common.bridge import (
    assert_bridgeout_receipt,
    derive_p2wpkh_bosd_hex,
    ee_log_path,
    submit_bridgeout_transaction,
    wait_for_output_snark_update,
)
from common.config.constants import ALPEN_ACCOUNT_ID, NEPAL_ACCOUNT_ID, SATS_TO_WEI, ServiceType
from common.evm_utils import subject_hex_from_address, wait_for_ee_balance
from common.ol_utils import (
    build_gam_tx,
    count_new_inbox_messages,
    get_ol_balance,
    wait_for_account_update_exact_seq,
    wait_for_inbox_message_delta,
    wait_for_next_ol_epoch,
    wait_for_ol_balance,
)
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.test_cli import build_snark_subject_transfer, create_mock_deposit
from envconfigs.el_ol import NEPAL_SEQUENCER_SERVICE

logger = logging.getLogger(__name__)

ALPEN_ACCOUNT_SERIAL = 128
NEPAL_ACCOUNT_SERIAL = 129

TRANSFER_AMOUNT_SATS = 100_000_000


@flexitest.register
class TestTwoAlpenEEs(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol_two_ees")

    def main(self, ctx):
        del ctx
        strata = cast(StrataService, self.get_service(ServiceType.Strata))
        bitcoin = cast(BitcoinService, self.get_service(ServiceType.Bitcoin))
        alpen = cast(AlpenClientService, self.get_service(ServiceType.AlpenSequencer))
        nepal = cast(AlpenClientService, self.get_service(NEPAL_SEQUENCER_SERVICE))

        strata_rpc = strata.wait_for_rpc_ready(timeout=30)
        submit_rpc = strata.create_submit_rpc()
        btc_rpc = bitcoin.create_rpc()
        alpen_rpc = alpen.create_rpc()
        nepal_rpc = nepal.create_rpc()

        strata.wait_for_account_genesis_epoch_commitment(ALPEN_ACCOUNT_ID, strata_rpc, timeout=30)
        strata.wait_for_account_genesis_epoch_commitment(NEPAL_ACCOUNT_ID, strata_rpc, timeout=30)

        alpen_recipient = Account.create()
        nepal_recipient = Account.create()
        alpen_subject = subject_hex_from_address(alpen_recipient.address)
        nepal_subject = subject_hex_from_address(nepal_recipient.address)

        miner_addr = btc_rpc.proxy.getnewaddress()
        deposit_amount_sats = 3 * TRANSFER_AMOUNT_SATS
        expected_deposit_wei = deposit_amount_sats * SATS_TO_WEI

        assert get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID) == 0
        assert get_ol_balance(strata_rpc, NEPAL_ACCOUNT_ID) == 0

        create_mock_deposit(
            account_serial=ALPEN_ACCOUNT_SERIAL,
            amount=deposit_amount_sats,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
            subject=alpen_subject,
        )
        btc_rpc.proxy.generatetoaddress(1, miner_addr)
        create_mock_deposit(
            account_serial=NEPAL_ACCOUNT_SERIAL,
            amount=deposit_amount_sats,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
            subject=nepal_subject,
        )
        btc_rpc.proxy.generatetoaddress(8, miner_addr)
        strata.wait_for_additional_blocks(
            2 * strata.props["slots_per_epoch"], strata_rpc, timeout_per_block=15
        )

        wait_for_ol_balance(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            deposit_amount_sats,
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
        )
        wait_for_ol_balance(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            deposit_amount_sats,
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
        )
        wait_for_ee_balance(
            alpen_rpc,
            btc_rpc,
            miner_addr,
            alpen_recipient.address,
            expected_deposit_wei,
        )
        wait_for_ee_balance(
            nepal_rpc,
            btc_rpc,
            miner_addr,
            nepal_recipient.address,
            expected_deposit_wei,
        )

        nepal_balance_before_gam = get_ol_balance(strata_rpc, NEPAL_ACCOUNT_ID)
        gam_start_slot = strata_rpc.strata_getChainStatus()["tip"]["slot"]
        gam_start_count = count_new_inbox_messages(strata_rpc, NEPAL_ACCOUNT_ID, gam_start_slot)
        submit_rpc.strata_submitTransaction(build_gam_tx(NEPAL_ACCOUNT_ID, "74776f2d65652d67616d"))
        wait_for_inbox_message_delta(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            gam_start_slot,
            gam_start_count,
            1,
            "nepal did not receive direct GAM inbox message",
        )
        if get_ol_balance(strata_rpc, NEPAL_ACCOUNT_ID) != nepal_balance_before_gam:
            raise AssertionError("zero-value GAM changed nepal OL balance")

        alpen_log = ee_log_path(alpen)
        nepal_log = ee_log_path(nepal)
        alpen_output_log_offset = alpen_log.stat().st_size if alpen_log.exists() else 0
        nepal_output_log_offset = nepal_log.stat().st_size if nepal_log.exists() else 0
        start_epoch = wait_for_next_ol_epoch(strata_rpc, btc_rpc, miner_addr)

        alpen_withdraw_hash = submit_bridgeout_transaction(
            alpen_rpc,
            alpen_recipient.address,
            alpen_recipient.key.hex(),
            derive_p2wpkh_bosd_hex(btc_rpc),
            TRANSFER_AMOUNT_SATS,
        )
        nepal_withdraw_hash = submit_bridgeout_transaction(
            nepal_rpc,
            nepal_recipient.address,
            nepal_recipient.key.hex(),
            derive_p2wpkh_bosd_hex(btc_rpc),
            TRANSFER_AMOUNT_SATS,
        )
        alpen_withdraw_gas_wei = assert_bridgeout_receipt(alpen_rpc, alpen_withdraw_hash)
        nepal_withdraw_gas_wei = assert_bridgeout_receipt(nepal_rpc, nepal_withdraw_hash)

        expected_alpen_ee_after_withdrawal = (
            expected_deposit_wei - TRANSFER_AMOUNT_SATS * SATS_TO_WEI - alpen_withdraw_gas_wei
        )
        expected_nepal_ee_after_withdrawal = (
            expected_deposit_wei - TRANSFER_AMOUNT_SATS * SATS_TO_WEI - nepal_withdraw_gas_wei
        )
        wait_for_ee_balance(
            alpen_rpc,
            btc_rpc,
            miner_addr,
            alpen_recipient.address,
            expected_alpen_ee_after_withdrawal,
        )
        wait_for_ee_balance(
            nepal_rpc,
            btc_rpc,
            miner_addr,
            nepal_recipient.address,
            expected_nepal_ee_after_withdrawal,
        )

        alpen_seq_no = wait_for_output_snark_update(
            alpen_log,
            btc_rpc,
            miner_addr,
            after_offset=alpen_output_log_offset,
        )
        nepal_seq_no = wait_for_output_snark_update(
            nepal_log,
            btc_rpc,
            miner_addr,
            after_offset=nepal_output_log_offset,
        )

        alpen_withdraw_epoch = wait_for_account_update_exact_seq(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            alpen_seq_no,
            start_epoch,
            btc_rpc,
            miner_addr,
        )
        nepal_withdraw_epoch = wait_for_account_update_exact_seq(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            nepal_seq_no,
            start_epoch,
            btc_rpc,
            miner_addr,
        )
        if alpen_withdraw_epoch != nepal_withdraw_epoch:
            raise AssertionError(
                f"withdrawals landed in different OL epochs: "
                f"alpen={alpen_withdraw_epoch}, nepal={nepal_withdraw_epoch}"
            )

        expected_alpen_after_withdrawal = deposit_amount_sats - TRANSFER_AMOUNT_SATS
        expected_nepal_after_withdrawal = deposit_amount_sats - TRANSFER_AMOUNT_SATS
        wait_for_ol_balance(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            expected_alpen_after_withdrawal,
            timeout=120,
        )
        wait_for_ol_balance(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            expected_nepal_after_withdrawal,
            timeout=120,
        )

        alpen.stop()

        alpen_balance_before_transfer = get_ol_balance(strata_rpc, ALPEN_ACCOUNT_ID)
        nepal_balance_before_transfer = get_ol_balance(strata_rpc, NEPAL_ACCOUNT_ID)
        expected_alpen_after_transfer = alpen_balance_before_transfer - TRANSFER_AMOUNT_SATS
        expected_nepal_after_transfer = nepal_balance_before_transfer + TRANSFER_AMOUNT_SATS

        transfer_start_slot = strata_rpc.strata_getChainStatus()["tip"]["slot"]
        transfer_start_count = count_new_inbox_messages(
            strata_rpc, NEPAL_ACCOUNT_ID, transfer_start_slot
        )
        alpen_state = strata_rpc.strata_getSnarkAccountStateByTag(ALPEN_ACCOUNT_ID, "latest")

        transfer_tx = build_snark_subject_transfer(
            target_hex=ALPEN_ACCOUNT_ID,
            seq_no=alpen_state["seq_no"],
            inner_state_hex=alpen_state["inner_state"],
            next_inbox_idx=alpen_state["next_inbox_msg_idx"],
            dest_account_hex=NEPAL_ACCOUNT_ID,
            source_subject_hex=alpen_subject,
            dest_subject_hex=nepal_subject,
            amount=TRANSFER_AMOUNT_SATS,
        )
        submit_rpc.strata_submitTransaction(transfer_tx)

        wait_for_ol_balance(
            strata_rpc,
            ALPEN_ACCOUNT_ID,
            expected_alpen_after_transfer,
            timeout=120,
        )
        wait_for_ol_balance(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            expected_nepal_after_transfer,
            timeout=120,
        )
        wait_for_inbox_message_delta(
            strata_rpc,
            NEPAL_ACCOUNT_ID,
            transfer_start_slot,
            transfer_start_count,
            1,
            "nepal did not receive cross-EE transfer inbox message",
        )
        wait_for_ee_balance(
            nepal_rpc,
            btc_rpc,
            miner_addr,
            nepal_recipient.address,
            expected_nepal_ee_after_withdrawal + TRANSFER_AMOUNT_SATS * SATS_TO_WEI,
        )

        logger.info("two-EE deposit, withdrawal, GAM, OL transfer, and EE mint checks passed")
        return True
