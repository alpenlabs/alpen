"""Shared assertions and L1-driving helpers for checkpoint-sync tests."""

from typing import cast

from common.rpc_types.strata import AccountEpochSummary, ChainSyncStatus
from common.services.strata import StrataService


def check_top_level_state_equivalent(
    seq_status: ChainSyncStatus, node_status: ChainSyncStatus
):
    """Checks CSS's top-level state at the shared finalized checkpoint.

    CSS publishes confirmed, latest, and finalized together at the terminal tip.
    """
    assert node_status["finalized"] == seq_status["finalized"], (
        "checkpoint-sync finalized commitment differs from sequencer: "
        f"css={node_status['finalized']} sequencer={seq_status['finalized']}"
    )

    finalized = node_status["finalized"]
    assert node_status["confirmed"] == finalized
    assert node_status["latest"] == finalized
    assert node_status["tip"] == {
        "epoch": finalized["epoch"],
        "slot": finalized["last_slot"],
        "blkid": finalized["last_blkid"],
        "is_terminal": True,
    }


def check_summaries_equivalent(seq_summary: AccountEpochSummary, node_summary: AccountEpochSummary):
    """Checks equivalent account summaries across block and checkpoint sync.

    Checkpoint reconstruction may omit non-terminal per-update state roots;
    when it does report one, it must equal the sequencer's root.
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


def mine_and_get_status(strata: StrataService, btc_rpc) -> ChainSyncStatus:
    """Mines L1 blocks so OL checkpoints confirm, then returns node status."""
    btc_rpc.proxy.generatetoaddress(2, btc_rpc.proxy.getnewaddress())
    status = strata.get_sync_status()
    check_epoch_ordering_invariant(status)
    return status


def check_epoch_ordering_invariant(status: ChainSyncStatus):
    """The latest epoch can never lag confirmed or finalized."""
    tip = status["tip"]["epoch"]
    latest = status["latest"]["epoch"]
    confirmed = status["confirmed"]["epoch"]
    finalized = status["finalized"]["epoch"]
    assert tip >= latest >= confirmed >= finalized, (
        f"epoch ordering violated: tip={tip}, latest={latest},"
        f"confirmed={confirmed}, finalized={finalized}"
    )
