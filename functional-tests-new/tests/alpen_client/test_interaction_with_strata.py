"""Tests that the alpen sequencer client is correctly syncing from strata,
producing blocks and posting updates"""

import logging
import time

from common.rpc_types.strata import AccountEpochSummary, EpochCommitment
from common.services.strata import StrataService
from common.wait import wait_until_with_value
import flexitest

from common.base_test import BaseTest
from common.services.alpen_client import AlpenClientService
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType

logger = logging.getLogger(__name__)

EXPECT_UPDATE_WITHIN_EPOCH = 5
CHECK_N_UPDATES = 3  # How many updates from alpen to check in strata

@flexitest.register
class TestAlpenSequencerToStrataSequencer(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service("alpen_sequencer")
        strata_seq: StrataService = self.get_service(ServiceType.Strata)

        # Wait for chains to be active
        logger.info("Waiting for Strata RPC to be ready...")
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=10)
        alpen_seq.wait_for_block(5, timeout=60)

        # Get alpen state at epoch 0
        acct_summary: AccountEpochSummary = strata_rpc.strata_getAccountEpochSummary(ALPEN_ACCOUNT_ID, 0)
        assert acct_summary["update_input"] is None, "No update input at epoch 0"

        last_new_update_at = 0
        new_updates_count = 0
        nxt_epoch = 1
        # Some future account summary should have update input
        while True:
            # Wait until next_epoch is present
            status = wait_until_with_value(
                strata_seq.get_sync_status,
                lambda s: s["confirmed"]["epoch"] >= nxt_epoch,
                error_with=f"Expected epoch {nxt_epoch} not found",
                timeout=10,
            )
            # Get account summary for new epochs because confirmed epoch might exceed what we expected
            for ep in range(nxt_epoch, status["confirmed"]["epoch"] + 1):
                acct_summary: AccountEpochSummary = strata_rpc.strata_getAccountEpochSummary(ALPEN_ACCOUNT_ID, ep)
                if acct_summary["update_input"] is not None:
                    logger.info(f"Received update input {new_updates_count + 1}. Alpen is submitting updates to strata. {acct_summary}")
                    last_new_update_at = ep
                    new_updates_count += 1
                elif ep > last_new_update_at + EXPECT_UPDATE_WITHIN_EPOCH:
                    assert False, "No new update received"
                nxt_epoch += 1

            if new_updates_count >= CHECK_N_UPDATES:
                break
