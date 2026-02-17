"""Tests that the alpen sequencer client is correctly syncing from strata,
producing blocks and posting updates"""

import logging

import flexitest

from common.base_test import BaseTest
from common.services.alpen_client import AlpenClientService

logger = logging.getLogger(__name__)

@flexitest.register
class TestAlpenSequencerToStrataSequencer(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service("alpen_sequencer")

        # Wait for chain to be active
        alpen_seq.wait_for_block(5, timeout=60)

        seq_rpc = alpen_seq.create_rpc()

        exec_status = seq_rpc.alpen_getExecChainStatus()
        logger.info(exec_status)
        ol_status = seq_rpc.alpen_getOLTrackingStatus()
        logger.info(ol_status)

        import time
        time.sleep(10)
        for i in range(10):
            exec_status = seq_rpc.alpen_getExecChainStatus()
            print("EXEC", exec_status)
            ol_status = seq_rpc.alpen_getOLTrackingStatus()
            print("OL", ol_status)
