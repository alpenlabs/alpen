"""CSS reconstructs real bridge deposit and withdrawal-intent updates."""

import logging
from typing import cast

import flexitest
from eth_account import Account

from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from tests.alpen_client.test_real_bridge_deposit_withdraw import (
    derive_p2wpkh_bosd_hex,
    ee_log_path,
    get_ol_balance,
    read_bridge_denomination,
    read_operator_xprivs,
    submit_bridgeout_and_wait_for_sau,
    submit_deposits_and_assert_ol_credit,
    wait_for_account_update_seq,
    wait_for_ee_balance_exact,
)
from tests.strata.test_checkpoint_sync_node import check_summaries_equivalent

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointSyncBridge(BaseTest):
    """Replays a real bridge deposit and withdrawal intent through CSS."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol_checkpoint_sync")

    def main(self, ctx):
        del ctx
        sequencer = cast(StrataService, self.get_service(ServiceType.Strata))
        checkpoint_node = cast(StrataService, self.get_service(ServiceType.StrataCheckpointNode))
        alpen = cast(AlpenClientService, self.get_service(ServiceType.AlpenSequencer))
        bitcoin = cast(BitcoinService, self.get_service(ServiceType.Bitcoin))
        seq_rpc = sequencer.wait_for_rpc_ready(timeout=30)
        checkpoint_node.wait_for_rpc_ready(timeout=30)
        btc_rpc = bitcoin.create_rpc()

        sequencer.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID, rpc=seq_rpc, timeout=30
        )
        operator_xprivs = read_operator_xprivs(sequencer)
        denomination = read_bridge_denomination(sequencer)
        recipient = Account.create()
        miner_addr = btc_rpc.proxy.getnewaddress()
        slots_per_epoch = sequencer.props.get("slots_per_epoch", 5)

        credited_sats = submit_deposits_and_assert_ol_credit(
            btc_rpc,
            seq_rpc,
            sequencer,
            operator_xprivs=operator_xprivs,
            recipient_addr_hex=recipient.address[2:].lower(),
            miner_addr=miner_addr,
            slots_per_epoch=slots_per_epoch,
            bridge_denom_sats=denomination,
            deposit_count=2,
        )
        assert get_ol_balance(seq_rpc, ALPEN_ACCOUNT_ID) == credited_sats
        alpen_rpc = alpen.create_rpc()
        wait_for_ee_balance_exact(
            alpen_rpc,
            btc_rpc,
            miner_addr,
            deposit_recipient_addr=recipient.address,
            expected_wei=credited_sats * 10**10,
        )

        _, recipient_bosd_hex = derive_p2wpkh_bosd_hex(btc_rpc)
        ee_log = ee_log_path(alpen)
        log_offset = ee_log.stat().st_size if ee_log.exists() else 0
        start_epoch = seq_rpc.strata_getChainStatus()["latest"]["epoch"]
        seq_no = submit_bridgeout_and_wait_for_sau(
            alpen_rpc,
            btc_rpc,
            ee_log,
            deposit_recipient_addr=recipient.address,
            deposit_recipient_privkey_hex=recipient.key.hex(),
            recipient_bosd_hex=recipient_bosd_hex,
            withdraw_sats=denomination,
            miner_addr=miner_addr,
            ee_output_log_offset=log_offset,
        )
        withdrawal_epoch = wait_for_account_update_seq(
            seq_rpc,
            ALPEN_ACCOUNT_ID,
            min_next_seq_no=seq_no,
            start_epoch=start_epoch,
            btc_rpc=btc_rpc,
            miner_addr=miner_addr,
        )

        wait_until_with_value(
            lambda: checkpoint_node.get_sync_status(),
            lambda status: status["finalized"]["epoch"] >= withdrawal_epoch,
            error_with="CSS did not finalize the bridge withdrawal epoch",
            timeout=180,
        )
        check_summaries_equivalent(
            sequencer.get_account_epoch_summary(ALPEN_ACCOUNT_ID, withdrawal_epoch),
            checkpoint_node.get_account_epoch_summary(ALPEN_ACCOUNT_ID, withdrawal_epoch),
        )
        logger.info(
            "CSS reproduced bridge deposit and withdrawal intent at epoch %d", withdrawal_epoch
        )
