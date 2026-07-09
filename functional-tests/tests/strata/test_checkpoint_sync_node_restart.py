"""A checkpoint-sync OL node restarts cleanly after syncing a post-genesis epoch.

A checkpoint-sync node stores no OL blocks except genesis. This test lets the
sequencer post checkpoints to L1, waits for the checkpoint-sync node to itself
finalize a post-genesis epoch (so it has persisted non-genesis client state),
then restarts that node reusing its datadir. The second startup must not require
the finalized OL block to be present in the store.
"""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.rpc_types.strata import ChainSyncStatus
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from tests.dbtool.helpers import get_mmr_leaf_count, load_genesis_height, run_dbtool_json

logger = logging.getLogger(__name__)

L1_BLOCK_REFS_MMR_ID = "l1-block-refs"


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

        # First get checkpoint node to sync up to first checkpoint.
        pre_restart_status = wait_until_with_value(
            lambda: mine_and_get_status(checkpoint_node, btc_rpc),
            lambda st: st["finalized"]["epoch"] >= 1,
            error_with="checkpoint-sync node did not finalize a post-genesis epoch",
            timeout=120,
        )
        finalized_epoch = pre_restart_status["finalized"]["epoch"]
        logger.info(f"checkpoint-sync node finalized epoch {finalized_epoch}; restarting")

        # Now restart. The node is stopped while dbtool observes the MMR index
        # because the live process owns the sled databases.
        datadir = checkpoint_node.props["datadir"]
        checkpoint_node.stop()
        genesis_l1_height = load_genesis_height(datadir)
        genesis_l1_leaf_count = genesis_l1_height + 1
        expected_l1_count = self._epoch_summary_l1_block_refs_count(datadir, finalized_epoch)
        pre_restart_l1_count = get_mmr_leaf_count(datadir, L1_BLOCK_REFS_MMR_ID)
        assert pre_restart_l1_count > genesis_l1_leaf_count, (
            f"checkpoint-sync node should have post-genesis L1 refs before restart "
            f"({pre_restart_l1_count} <= {genesis_l1_leaf_count})"
        )
        assert pre_restart_l1_count == expected_l1_count, (
            f"checkpoint-sync L1 refs MMR should match epoch {finalized_epoch} summary "
            f"before restart ({pre_restart_l1_count} != {expected_l1_count})"
        )

        checkpoint_node.start()
        checkpoint_node.wait_for_rpc_ready(timeout=30)

        post_restart_status = checkpoint_node.get_sync_status()
        tip_slot = post_restart_status["tip"]["slot"]
        logger.info(f"checkpoint-sync node restarted; canonical tip at slot {tip_slot}")

        checkpoint_node.stop()
        restarted_l1_count = get_mmr_leaf_count(datadir, L1_BLOCK_REFS_MMR_ID)
        assert restarted_l1_count == expected_l1_count, (
            f"checkpoint-sync L1 refs MMR should match epoch {finalized_epoch} summary "
            f"after restart ({restarted_l1_count} != {expected_l1_count})"
        )
        checkpoint_node.start()
        checkpoint_node.wait_for_rpc_ready(timeout=30)

        # Require a strictly new finalization after restart.
        wait_until_with_value(
            lambda: mine_and_get_status(checkpoint_node, btc_rpc),
            lambda st: st["finalized"]["epoch"] > finalized_epoch,
            error_with="checkpoint-sync node did not finalize a new epoch after restart",
            timeout=120,
        )

        checkpoint_node.stop()
        post_restart_l1_count = get_mmr_leaf_count(datadir, L1_BLOCK_REFS_MMR_ID)
        assert post_restart_l1_count >= pre_restart_l1_count, (
            f"checkpoint-sync L1 refs MMR regressed across restart "
            f"({post_restart_l1_count} < {pre_restart_l1_count})"
        )

        # This env is shared by name with other checkpoint-sync tests, so leave
        # the node running: the final stop() above was only to release the sled
        # lock for the offline dbtool read.
        checkpoint_node.start()
        checkpoint_node.wait_for_rpc_ready(timeout=30)

    @staticmethod
    def _epoch_summary_l1_block_refs_count(datadir: str, epoch: int) -> int:
        """Returns expected L1-block-refs leaf count for an epoch summary."""
        summary = run_dbtool_json(datadir, "get-epoch-summary", str(epoch))
        new_l1_height = int(summary["epoch_summary"]["new_l1"]["height"])
        return new_l1_height + 1
