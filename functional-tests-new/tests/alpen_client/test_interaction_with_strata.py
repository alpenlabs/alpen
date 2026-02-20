"""Tests that the alpen sequencer client is correctly syncing from strata,
producing blocks and posting updates"""

import logging

import flexitest

from common.rpc_types.strata import AccountEpochSummary
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from common.base_test import BaseTest
from common.services.alpen_client import AlpenClientService
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType

logger = logging.getLogger(__name__)

# This is empirical and is used to allow alpen to create and submit DA and get it confirmed.
# TODO: might need to more intelligently calculate this
EXPECT_UPDATE_WITHIN_EPOCH = 12
CHECK_N_UPDATES = 3  # How many updates from alpen to check in strata


@flexitest.register
class TestAlpenSequencerToStrataSequencer(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service("alpen_sequencer")
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        # Wait for chains to be active
        logger.info("Waiting for Strata RPC to be ready...")
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=10)
        alpen_seq.wait_for_block(5, timeout=60)

        # Get alpen account summary at epoch 0 which should be none
        acct_summary: AccountEpochSummary = strata_rpc.strata_getAccountEpochSummary(
            ALPEN_ACCOUNT_ID, 0
        )
        assert acct_summary["update_input"] is None, "No update input at epoch 0"

        last_new_update_at = 0
        new_updates_count = 0
        next_epoch = 1

        while new_updates_count < CHECK_N_UPDATES:
            # Wait until next_epoch is present
            status = wait_until_with_value(
                lambda: get_sync_status_and_mine_blocks(strata_seq, btc_rpc),
                lambda s: s["confirmed"]["epoch"] >= next_epoch,
                error_with=f"Expected epoch {next_epoch} not found",
                timeout=10,
            )
            new_epochs_since_last = range(next_epoch, status["confirmed"]["epoch"] + 1)

            # Check for new updates in one of the new epochs
            for ep in new_epochs_since_last:
                acct_summary: AccountEpochSummary = strata_rpc.strata_getAccountEpochSummary(
                    ALPEN_ACCOUNT_ID, ep
                )

                if acct_summary["update_input"] is not None:
                    logger.info(
                        f"Received update input {new_updates_count + 1}. Alpen is submitting updates to strata. {acct_summary}"
                    )
                    last_new_update_at = ep
                    new_updates_count += 1

                elif ep > last_new_update_at + EXPECT_UPDATE_WITHIN_EPOCH:
                    assert False, (
                        f"No new update(nth={new_updates_count + 1}) received within {EXPECT_UPDATE_WITHIN_EPOCH} epochs"
                    )

                next_epoch += 1


def get_sync_status_and_mine_blocks(strata: StrataService, btc_rpc):
    """Gets sync status, but also piggybacks block mining to let DA chunks submitted by alpen to get included"""
    mine_address = btc_rpc.proxy.getnewaddress()
    btc_rpc.proxy.generatetoaddress(2, mine_address)
    return strata.get_sync_status()
