"""The terminal-header backfill is successful and idempotent on an eager datadir."""

import logging

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.services.strata import StrataService
from envconfigs.el_ol_checkpoint_sync import EeOLCheckpointSyncEnv
from tests.dbtool.helpers import extract_json_from_output, run_dbtool
from tests.strata.checkpoint_promotion import finalize_active_checkpoint

logger = logging.getLogger(__name__)


@flexitest.register
class TestCheckpointTerminalHeaderBackfill(BaseTest):
    """Runs backfill twice after eager checkpoint application wrote all headers."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            EeOLCheckpointSyncEnv(
                pre_generate_blocks=110,
                seal_epoch_slots=4,
                ol_block_time_ms=750,
                l1_reorg_safe_depth=4,
                batch_sealing_block_count=3,
            )
        )

    def main(self, ctx):
        anchor = finalize_active_checkpoint(self)
        checkpoint_node: StrataService = self.get_service(ServiceType.StrataCheckpointNode)
        checkpoint_node.stop()

        datadir = checkpoint_node.props["datadir"]
        first = self._run_backfill(datadir)
        second = self._run_backfill(datadir)
        for report in (first, second):
            assert report["epochs_scanned"] >= anchor.epoch
            assert report["headers_written"] == 0
            assert report["headers_skipped"] == report["epochs_scanned"]
            assert report["headers_not_backfilled"] == 0
            assert report["missing_observed_payload_epochs"] == []
        assert second == first
        logger.info(
            "terminal-header backfill skipped all %s eager records twice", first["epochs_scanned"]
        )
        return True

    @staticmethod
    def _run_backfill(datadir: str) -> dict:
        code, stdout, stderr = run_dbtool(
            datadir,
            "backfill-terminal-headers",
            "-o",
            "json",
        )
        assert code == 0, stderr or stdout
        return extract_json_from_output(stdout)
