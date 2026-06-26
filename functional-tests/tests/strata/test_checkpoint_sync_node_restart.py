"""A checkpoint-sync OL node restarts cleanly after syncing a post-genesis epoch.

A checkpoint-sync node stores no OL blocks except genesis: it persists OL state
and epoch summaries when applying checkpoints, and the finalized epoch lives in
client state. Once it has synced a post-genesis checkpoint, its persisted client
state declares a finalized epoch whose OL block is not in the store.

This test lets the sequencer post checkpoints to L1, waits for the
checkpoint-sync node to itself finalize a post-genesis epoch (so it has persisted
non-genesis client state), then restarts that node reusing its datadir. The
second startup must not require the finalized OL block to be present in the
store.
"""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.rpc_types.strata import ChainSyncStatus
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)


def mine_and_get_status(strata: StrataService, btc_rpc) -> ChainSyncStatus:
    """Mines L1 blocks so OL checkpoints confirm, then returns the node's status."""
    btc_rpc.proxy.generatetoaddress(2, btc_rpc.proxy.getnewaddress())
    return strata.get_sync_status()


@flexitest.register
class TestCheckpointSyncNodeRestart(BaseTest):
    """Restarts a checkpoint-sync node after it syncs a post-genesis epoch."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol_checkpoint_sync")

    def main(self, ctx):
        checkpoint_node: StrataService = self.get_service(ServiceType.StrataCheckpointNode)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        checkpoint_node.wait_for_rpc_ready(timeout=20)

        # First get checkpoint node to sync upto first chekpoint.
        pre_restart_status = wait_until_with_value(
            lambda: mine_and_get_status(checkpoint_node, btc_rpc),
            lambda st: st["finalized"]["epoch"] >= 1,
            error_with="checkpoint-sync node did not finalize a post-genesis epoch",
            timeout=120,
        )
        finalized_epoch = pre_restart_status["finalized"]["epoch"]
        logger.info(f"checkpoint-sync node finalized epoch {finalized_epoch}; restarting")

        # Now restart.
        checkpoint_node.stop()
        checkpoint_node.start()
        checkpoint_node.wait_for_rpc_ready(timeout=30)

        post_restart_status = checkpoint_node.get_sync_status()
        tip_slot = post_restart_status["tip"]["slot"]
        logger.info(f"checkpoint-sync node restarted; canonical tip at slot {tip_slot}")
        wait_until_with_value(
            lambda: mine_and_get_status(checkpoint_node, btc_rpc),
            lambda st: st["finalized"]["epoch"] >= 2,
            error_with="checkpoint-sync node did not finalize a post-genesis epoch",
            timeout=120,
        )
