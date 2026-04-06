"""Tests that the alpen sequencer client is correctly syncing from strata,
producing blocks and posting updates"""

import logging

from common.utils import with_mining
import flexitest

from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.rpc_types.strata import AccountEpochSummary
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

# This is empirical and is used to allow alpen to create and submit DA and get it confirmed.
# TODO: might need to more intelligently calculate this
EXPECT_UPDATE_WITHIN_EPOCH = 10
CHECK_N_UPDATES = 3  # How many updates from alpen to check in strata
NUM_INIT_BLOCKS = 5


@flexitest.register
class TestAlpenSequencerToStrataSequencer(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        strata_node: StrataService = self.get_service(ServiceType.StrataNode)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        # Wait for chains to be active
        logger.info("Waiting for Strata RPC to be ready...")
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=10)
        node_rpc = strata_node.wait_for_rpc_ready(timeout=10)

        logger.info("Waiting for Alpen account genesis commitment...")
        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,
            rpc=strata_rpc,
            timeout=20,
        )
        alpen_seq.wait_for_block(5, timeout=60)
        logger.info("generated initial blocks")

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
                with_mining(btc_rpc, strata_seq.get_sync_status),
                lambda s, next_epoch=next_epoch: s["tip"]["epoch"] > next_epoch,
                error_with=f"Expected epoch {next_epoch} not found",
                timeout=60,
            )

            tip_epoch = status["tip"]["epoch"]

            # Wait until tip epoch is finalized in sequencer and strata node
            strata_seq.wait_until_checkpoint_finalized(tip_epoch, btcrpc=btc_rpc)
            strata_node.wait_until_checkpoint_finalized(tip_epoch)

            new_epochs_since_last = list(range(next_epoch, tip_epoch))
            logger.info(f"new epochs since last: {new_epochs_since_last}")

            # Check for new updates in one of the new epochs
            for ep in new_epochs_since_last:
                acct_summary: AccountEpochSummary = strata_rpc.strata_getAccountEpochSummary(
                    ALPEN_ACCOUNT_ID, ep
                )

                if acct_summary["update_input"] is not None:
                    logger.info(
                        f"Received update input {new_updates_count + 1}. "
                        f"Alpen is submitting updates to Strata. {acct_summary}"
                    )
                    last_new_update_at = ep
                    new_updates_count += 1

                    # Check that with correponding result from strata node
                    node_acct_summary = node_rpc.strata_getAccountEpochSummary(ALPEN_ACCOUNT_ID, ep)
                    assert node_acct_summary == acct_summary, (
                        f"Account summary for epoch {ep} should be same "
                        "from sequencer and checkpoint sync node"
                    )

                elif ep > last_new_update_at + EXPECT_UPDATE_WITHIN_EPOCH:
                    raise AssertionError(
                        f"No new update (nth={new_updates_count + 1}) received"
                        f" within {EXPECT_UPDATE_WITHIN_EPOCH} epochs"
                    )

                next_epoch += 1
