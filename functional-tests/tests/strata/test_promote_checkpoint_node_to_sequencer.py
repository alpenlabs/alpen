"""A nuked sequencer is replaced by promoting a checkpoint-sync datadir."""

import flexitest

from common.base_test import BaseTest
from envconfigs.el_ol_checkpoint_sync import EeOLCheckpointSyncEnv
from tests.strata.checkpoint_promotion import (
    assert_fresh_checkpoint_recovery,
    assert_sequencing_resumed,
    finalize_active_checkpoint,
    finalize_promoted_epoch,
    nuke_sequencer_and_promote,
)


@flexitest.register
class TestPromoteCheckpointNodeToSequencer(BaseTest):
    """Runs the STR-3820 full sequencer disaster-recovery scenario."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            EeOLCheckpointSyncEnv(
                pre_generate_blocks=110,
                seal_epoch_slots=4,
                ol_block_time_ms=750,
                l1_reorg_safe_depth=4,
                batch_sealing_block_count=3,
                provision_promotion=True,
                provision_recovery_node=True,
            )
        )

    def main(self, ctx):
        anchor = finalize_active_checkpoint(self)
        promoted = nuke_sequencer_and_promote(self, anchor)
        assert_sequencing_resumed(anchor, promoted)
        promoted_epoch = finalize_promoted_epoch(self, anchor, promoted)
        assert_fresh_checkpoint_recovery(self, promoted, promoted_epoch)
        return True
