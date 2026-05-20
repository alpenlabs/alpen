"""End-to-end exercise of the prover-task admin commands.

Operates entirely on the offline datadir: backfills a synthetic task via
the raw-key escape hatch, then walks it through the abandon / reset /
delete verbs and asserts each transition lands in the DB as documented.

We deliberately don't drive the prover here — the value of this test is
to lock in the DB-level admin contract, which is the surface STR-3414
delivers.
"""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from envconfigs.strata import StrataEnvConfig
from tests.dbtool.helpers import run_dbtool, run_dbtool_json

logger = logging.getLogger(__name__)

# Arbitrary hex key — the dbtool's raw backfill accepts any byte string,
# so we don't need to construct a real `CheckpointProofTask` here. The
# typed-backfill path is exercised separately by callers that have a
# canonical epoch commitment to resolve.
_RAW_KEY_A = "deadbeef"
_RAW_KEY_B = "cafebabe"


def _summary(datadir: str) -> dict:
    return run_dbtool_json(datadir, "get-prover-tasks-summary", "--limit", "100")


def _task(datadir: str, key_hex: str) -> dict:
    return run_dbtool_json(datadir, "get-prover-task", key_hex)


@flexitest.register
class DbtoolProverTaskAdminTest(StrataNodeTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=10,
                epoch_sealing=EpochSealingConfig.new_fixed_slot(4),
            )
        )

    def main(self, ctx):
        seq_service = self.get_service(ServiceType.Strata)
        seq_service.wait_for_rpc_ready(timeout=20)
        seq_service.stop()
        datadir = seq_service.props["datadir"]

        # Baseline: no prover tasks in a fresh node datadir.
        summary = _summary(datadir)
        assert summary["total"] == 0, summary

        # --confirm gating: every mutate must refuse without --confirm.
        for cmd in (
            ("backfill-prover-task-raw", _RAW_KEY_A),
            ("abandon-prover-task", _RAW_KEY_A),
            ("reset-prover-task", _RAW_KEY_A),
            ("delete-prover-task", _RAW_KEY_A),
        ):
            code, _, stderr = run_dbtool(datadir, *cmd)
            assert code != 0, f"expected {cmd[0]} to refuse without --confirm"
            assert "--confirm is required" in stderr, stderr

        # Backfill two raw tasks; both should land in Pending.
        for key in (_RAW_KEY_A, _RAW_KEY_B):
            code, _, stderr = run_dbtool(datadir, "backfill-prover-task-raw", key, "--confirm")
            assert code == 0, stderr

        summary = _summary(datadir)
        assert summary["total"] == 2, summary
        assert summary["pending"] == 2, summary

        # Abandon the first → PermanentFailure with the documented reason.
        code, _, stderr = run_dbtool(datadir, "abandon-prover-task", _RAW_KEY_A, "--confirm")
        assert code == 0, stderr
        record = _task(datadir, _RAW_KEY_A)
        assert record["status"]["name"] == "permanent_failure", record
        assert record["status"]["error"] == "abandoned via dbtool", record

        # Reset moves the second back to Pending and clears any retry_after.
        code, _, stderr = run_dbtool(datadir, "reset-prover-task", _RAW_KEY_B, "--confirm")
        assert code == 0, stderr
        record = _task(datadir, _RAW_KEY_B)
        assert record["status"]["name"] == "pending", record
        assert "retry_after_secs" not in record, record

        # Delete the abandoned one; it should drop from the summary.
        code, _, stderr = run_dbtool(datadir, "delete-prover-task", _RAW_KEY_A, "--confirm")
        assert code == 0, stderr
        summary = _summary(datadir)
        assert summary["total"] == 1, summary
        assert summary["pending"] == 1, summary

        # Bulk dry-run prints intent without writing.
        code, stdout, stderr = run_dbtool(
            datadir,
            "abandon-prover-tasks",
            "--all-unfinished",
            "--dry-run",
        )
        assert code == 0, stderr
        assert "would abandon" in stdout, stdout
        summary = _summary(datadir)
        assert summary["pending"] == 1, summary

        # Bulk for-real flips the remaining task to permanent_failure.
        code, _, stderr = run_dbtool(
            datadir,
            "abandon-prover-tasks",
            "--all-unfinished",
            "--confirm",
        )
        assert code == 0, stderr
        summary = _summary(datadir)
        assert summary["pending"] == 0, summary
        assert summary["permanent_failure"] == 1, summary

        return True
