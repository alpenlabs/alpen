"""Verify Alpen EE (reth-backed) survives restart while interacting with Strata."""

import logging
import time

import flexitest

from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.rpc_types.strata import AccountEpochSummary
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


@flexitest.register
class TestRethRestartWithStrata(BaseTest):
    """Restart Alpen EE sequencer service and verify EE + OL interaction remains healthy."""

    RESTART_PAUSE_SECONDS = 2
    MIN_EE_BLOCKS_BEFORE_RESTART = 5
    UPDATE_WAIT_TIMEOUT = 120
    MINE_BLOCKS_PER_POLL = 2

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        # Wait for Strata + Alpen EE to come online and establish initial sync.
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=20)
        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,
            rpc=strata_rpc,
            timeout=30,
        )
        alpen_seq.wait_for_block(self.MIN_EE_BLOCKS_BEFORE_RESTART, timeout=90)

        # Confirm at least one Alpen update reaches Strata before restart.
        first_update_epoch = self.wait_for_update_epoch(
            strata_seq,
            strata_rpc,
            btc_rpc,
            start_epoch=1,
        )
        logger.info("Observed first Alpen update at epoch %s", first_update_epoch)

        pre_restart_ee_height = alpen_seq.get_block_number()
        logger.info("EE height before restart: %s", pre_restart_ee_height)

        # Restart Alpen EE sequencer service (reth-backed process).
        alpen_seq.stop()
        time.sleep(self.RESTART_PAUSE_SECONDS)
        alpen_seq.start()
        alpen_seq.wait_for_ready(timeout=60)

        # Verify EE (reth RPC) is alive and still producing blocks.
        alpen_seq.wait_for_block(pre_restart_ee_height + 1, timeout=90)
        post_restart_ee_height = alpen_seq.get_block_number()
        assert post_restart_ee_height > pre_restart_ee_height, (
            "Expected EE to progress after restart "
            f"(before={pre_restart_ee_height}, after={post_restart_ee_height})"
        )
        logger.info("EE survived restart and progressed to height %s", post_restart_ee_height)

        # Verify Alpen continues submitting updates to Strata after restart.
        second_update_epoch = self.wait_for_update_epoch(
            strata_seq,
            strata_rpc,
            btc_rpc,
            start_epoch=first_update_epoch + 1,
        )
        logger.info("Observed second Alpen update at epoch %s", second_update_epoch)

        assert second_update_epoch > first_update_epoch, (
            "Expected post-restart Alpen update at a higher epoch "
            f"(first={first_update_epoch}, second={second_update_epoch})"
        )
        return True

    def wait_for_update_epoch(
        self,
        strata_seq: StrataService,
        strata_rpc,
        btc_rpc,
        start_epoch: int,
    ) -> int:
        """Mine L1 blocks and wait until Strata sees an Alpen update at/after `start_epoch`."""
        mine_address = btc_rpc.proxy.getnewaddress()

        def find_update_epoch() -> int | None:
            btc_rpc.proxy.generatetoaddress(self.MINE_BLOCKS_PER_POLL, mine_address)
            status = strata_seq.get_sync_status(strata_rpc)
            tip_epoch = status["tip"]["epoch"]

            if tip_epoch <= start_epoch:
                return None

            for epoch in range(start_epoch, tip_epoch):
                acct_summary: AccountEpochSummary = strata_rpc.strata_getAccountEpochSummary(
                    ALPEN_ACCOUNT_ID, epoch
                )
                if acct_summary["update_input"] is not None:
                    return epoch
            return None

        update_epoch = wait_until_with_value(
            find_update_epoch,
            lambda epoch: epoch is not None,
            error_with=f"Timed out waiting for Alpen update from epoch {start_epoch}",
            timeout=self.UPDATE_WAIT_TIMEOUT,
            step=1.0,
        )
        return int(update_epoch)
