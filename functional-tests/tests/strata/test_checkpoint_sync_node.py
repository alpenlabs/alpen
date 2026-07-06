"""A checkpoint-sync OL node reconstructs the same OL state as the sequencer.

The checkpoint-sync node syncs purely from L1-buried checkpoints, with no peer
OL connection. The test drives real account activity on the sequencer (via the
EE node) and asserts the checkpoint-sync node reconstructs identical per-epoch
account state.
"""

import logging
from typing import cast

import flexitest

from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.rpc_types.strata import AccountEpochSummary, ChainSyncStatus
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

# Number of epochs with real account activity to compare between the two nodes.
EPOCHS_WITH_ACTIVITY_TO_CHECK = 2
# Cap on how many epochs to walk while looking for activity.
MAX_EPOCHS_TO_SCAN = 30


@flexitest.register
class TestCheckpointSyncNode(BaseTest):
    """
    Tests a checkpoint syncing node. The EE node reads OL state from the
    checkpoint-sync node and submits transactions to the sequencer; the test
    asserts that per-epoch account summaries from the checkpoint-sync node
    match the sequencer's.
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

        # Each active epoch's reconstructed account summary must be identical to
        # the sequencer's, including the non-empty update inputs.
        seq_rpc = sequencer.create_rpc()
        for epoch in active_epochs:
            seq_summary = sequencer.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch)
            node_summary = checkpoint_node.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch)
            check_summaries_equivalent(seq_summary, node_summary)
            check_commitment_matches_checkpoint(seq_rpc, epoch, node_summary["epoch_commitment"])
            logger.info(f"account epoch summary matches at epoch {epoch}")


def check_summaries_equivalent(seq_summary: AccountEpochSummary, node_summary: AccountEpochSummary):
    """Checks that the two summaries match. The checkpoint-sync node may report
    `new_state_root=None` for non-terminal updates within a multi-update epoch
    (DA reconstruction only recovers terminal-per-account roots); when present,
    the root must match the sequencer's.
    """
    seq_summary_d = dict(seq_summary)
    node_summary_d = dict(node_summary)
    seq_updates = cast(list, seq_summary_d.pop("update_inputs"))
    node_updates = cast(list, node_summary_d.pop("update_inputs"))

    assert seq_summary_d == node_summary_d

    for su, nu in zip(seq_updates, node_updates, strict=True):
        s_root = su.pop("new_state_root")
        n_root = nu.pop("new_state_root")
        assert n_root is None or n_root == s_root, "new_state_root if present must match"
        assert su == nu


def check_commitment_matches_checkpoint(seq_rpc, epoch: int, commitment: dict):
    """Anchors the reconstructed epoch commitment to the published checkpoint.

    The terminal blkid hashes the reconstructed header (which commits to
    state_root), so equality proves replay yielded the expected post-state.
    """
    info = seq_rpc.call("strata_getCheckpointInfo", epoch)
    assert info is not None, f"missing checkpoint info at epoch {epoch}"
    terminal = info["l2_range"][1]
    assert commitment["last_slot"] == terminal["slot"], (
        f"epoch {epoch} commitment slot {commitment['last_slot']} != "
        f"checkpoint terminal slot {terminal['slot']}"
    )
    assert commitment["last_blkid"] == terminal["blkid"], (
        f"epoch {epoch} commitment blkid {commitment['last_blkid']} != "
        f"checkpoint terminal blkid {terminal['blkid']}"
    )


def mine_and_get_status(strata: StrataService, btc_rpc) -> ChainSyncStatus:
    """Mines L1 blocks so OL checkpoints confirm, then returns the node's status."""
    btc_rpc.proxy.generatetoaddress(2, btc_rpc.proxy.getnewaddress())
    return strata.get_sync_status()
