"""A checkpoint-sync OL node reconstructs the same OL state as the sequencer.

The checkpoint-sync node syncs purely from L1-buried checkpoints, with no peer
OL connection. The test drives real account activity on the sequencer (via the
EE node) and asserts the checkpoint-sync node reconstructs identical per-epoch
account state.
"""

import logging

import flexitest

from common.base_test import BaseTest
from common.checkpoint_sync import (
    check_summaries_equivalent,
    check_top_level_state_equivalent,
    mine_and_get_status,
)
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.rpc_types.strata import EpochCommitment
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

# Number of epochs with real account activity to compare between the two nodes.
EPOCHS_WITH_ACTIVITY_TO_CHECK = 5
# Cap on how many epochs to walk while looking for activity.
MAX_EPOCHS_TO_SCAN = 30


@flexitest.register
class TestCheckpointSyncNode(BaseTest):
    """
    Tests a checkpoint syncing node. The EE sequencer generates activity via
    the OL sequencer; the test asserts that CSS reconstructs matching
    per-epoch account summaries from L1 checkpoints.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol_checkpoint_sync")

    def main(self, ctx):
        sequencer: StrataService = self.get_service(ServiceType.Strata)
        checkpoint_node: StrataService = self.get_service(ServiceType.StrataCheckpointNode)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        # Wait for rpcs to be ready.
        sequencer.wait_for_rpc_ready(timeout=20)
        checkpoint_node.wait_for_rpc_ready(timeout=20)

        # Walk epochs as the EE node posts updates, collecting epochs whose EE
        # account summary on the sequencer has real activity.
        active_epochs: list[int] = []
        next_epoch = 1
        while len(active_epochs) < EPOCHS_WITH_ACTIVITY_TO_CHECK:
            if next_epoch > MAX_EPOCHS_TO_SCAN:
                raise AssertionError(
                    f"only found {len(active_epochs)} active epochs within "
                    f"{MAX_EPOCHS_TO_SCAN} epochs"
                )

            seq_status = wait_until_with_value(
                lambda: mine_and_get_status(sequencer, btc_rpc),
                lambda st, ep=next_epoch: st["tip"]["epoch"] > ep,
                error_with=f"sequencer did not advance past epoch {next_epoch}",
                timeout=120,
            )

            for epoch in range(next_epoch, seq_status["tip"]["epoch"]):
                summary = sequencer.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch)
                # Add to active epochs if updates are present for the account
                if len(summary["update_inputs"]) > 0:
                    active_epochs.append(epoch)
                    logger.info(f"epoch {epoch} has account activity")
            next_epoch = seq_status["tip"]["epoch"]

        last_active = active_epochs[-1]
        logger.info(f"comparing checkpoint-sync node up to epoch {last_active}")

        # The checkpoint-sync node reconstructs state from L1; wait for it to
        # finalize the last active epoch.
        wait_until_with_value(
            lambda: mine_and_get_status(checkpoint_node, btc_rpc),
            lambda st: st["finalized"]["epoch"] >= last_active,
            error_with=f"checkpoint-sync node did not finalize epoch {last_active}",
            timeout=120,
        )

        seq_status, node_status = wait_until_with_value(
            lambda: (sequencer.get_sync_status(), checkpoint_node.get_sync_status()),
            lambda statuses: statuses[0]["finalized"] == statuses[1]["finalized"],
            error_with="sequencer and checkpoint-sync finalized commitments did not converge",
            timeout=120,
        )
        check_top_level_state_equivalent(seq_status, node_status)

        # Each active epoch's reconstructed account summary must be identical to
        # the sequencer's, including the non-empty update inputs.
        seq_rpc = sequencer.create_rpc()
        for epoch in active_epochs:
            seq_summary = sequencer.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch)
            node_summary = checkpoint_node.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch)
            check_summaries_equivalent(seq_summary, node_summary)
            check_commitment_matches_checkpoint(seq_rpc, epoch, node_summary["epoch_commitment"])
            logger.info(f"account epoch summary matches at epoch {epoch}")


def check_commitment_matches_checkpoint(seq_rpc, epoch: int, commitment: EpochCommitment):
    """Anchors the reconstructed epoch commitment to the published checkpoint.

    The terminal blkid hashes the reconstructed header (which commits to
    state_root), so equality proves replay yielded the expected post-state.
    """
    info = seq_rpc.strata_getCheckpointInfo(epoch)
    assert info is not None, f"missing checkpoint info at epoch {epoch}"
    terminal = info["l2_end"]
    assert commitment["last_slot"] == terminal["slot"], (
        f"epoch {epoch} commitment slot {commitment['last_slot']} != "
        f"checkpoint terminal slot {terminal['slot']}"
    )
    assert commitment["last_blkid"] == terminal["blkid"], (
        f"epoch {epoch} commitment blkid {commitment['last_blkid']} != "
        f"checkpoint terminal blkid {terminal['blkid']}"
    )
