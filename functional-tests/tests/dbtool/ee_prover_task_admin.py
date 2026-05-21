"""End-to-end exercise of the EE prover-task admin commands.

Mirrors `prover_task_admin.py` but drives the alpen-client's prover
store (`<ee-datadir>/sled`) via `--ee-datadir`. We deliberately don't
drive the chunk/acct provers — the value of this test is to lock in
the DB-level admin contract, kind-tag filtering, and the dry-run-by-
default `--force` semantics that the dbtool delivers.
"""

import logging

import flexitest

from common.base_test import AlpenClientTest
from common.config import ServiceType
from tests.dbtool.helpers import run_dbtool_ee, run_dbtool_ee_json

logger = logging.getLogger(__name__)

# Kind-tagged raw keys. The alpen-client's task encoders prefix every
# key with `b'c'` (chunk) or `b'a'` (acct); we mirror that here so
# `--kind` filters land correctly.
_CHUNK_KEY = "63" + "11" * 8  # b'c' + 8 arbitrary bytes
_ACCT_KEY = "61" + "22" * 8  # b'a' + 8 arbitrary bytes


def _summary(ee_datadir: str, *extra: str) -> dict:
    return run_dbtool_ee_json(ee_datadir, "ee-get-prover-tasks-summary", "--limit", "100", *extra)


def _task(ee_datadir: str, key_hex: str) -> dict:
    return run_dbtool_ee_json(ee_datadir, "ee-get-prover-task", key_hex)


@flexitest.register
class DbtoolEeProverTaskAdminTest(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        seq_service = self.get_service(ServiceType.AlpenSequencer)
        seq_service.wait_for_ready(timeout=60)
        seq_service.stop()
        ee_datadir = seq_service.props["datadir"]

        # The alpen-client may already have written real chunk/acct
        # tasks before we stopped it; record the baseline and reason
        # in deltas so the test stays correct under any starting state.
        baseline = _summary(ee_datadir)
        baseline_total = baseline["total"]
        baseline_pending = baseline["pending"]
        baseline_permanent = baseline["permanent_failure"]

        # Dry-run-by-default: without --force, the backfill must print
        # "would backfill" + force hint and leave the DB unchanged.
        code, stdout, stderr = run_dbtool_ee(ee_datadir, "ee-backfill-prover-task-raw", _CHUNK_KEY)
        assert code == 0, stderr
        assert "would backfill" in stdout, stdout
        assert "Use --force to execute these changes." in stdout, stdout
        assert _summary(ee_datadir)["total"] == baseline_total, "dry-run backfill must not write"

        # Backfill one chunk-tagged and one acct-tagged task for real;
        # both should land in Pending.
        for key in (_CHUNK_KEY, _ACCT_KEY):
            code, _, stderr = run_dbtool_ee(
                ee_datadir, "ee-backfill-prover-task-raw", key, "--force"
            )
            assert code == 0, stderr

        after_backfill = _summary(ee_datadir)
        assert after_backfill["total"] == baseline_total + 2, after_backfill
        assert after_backfill["pending"] == baseline_pending + 2, after_backfill

        # Kind filters: the chunk-tagged key shows up under --kind chunk,
        # the acct-tagged key under --kind acct, and neither leaks.
        chunk_summary = _summary(ee_datadir, "--kind", "chunk")
        acct_summary = _summary(ee_datadir, "--kind", "acct")
        chunk_keys = {entry["key_hex"] for entry in chunk_summary["entries"]}
        acct_keys = {entry["key_hex"] for entry in acct_summary["entries"]}
        assert _CHUNK_KEY in chunk_keys, chunk_summary
        assert _ACCT_KEY not in chunk_keys, chunk_summary
        assert _ACCT_KEY in acct_keys, acct_summary
        assert _CHUNK_KEY not in acct_keys, acct_summary

        # Each surfaced entry should carry its derived kind label, which
        # is the field other admin tooling can grep on.
        for entry in chunk_summary["entries"]:
            if entry["key_hex"] == _CHUNK_KEY:
                assert entry["kind"] == "chunk", entry
        for entry in acct_summary["entries"]:
            if entry["key_hex"] == _ACCT_KEY:
                assert entry["kind"] == "acct", entry

        # Single-row dry runs against existing keys preview the action
        # and emit the same force hint, without writing.
        for verb in (
            "ee-abandon-prover-task",
            "ee-reset-prover-task",
            "ee-delete-prover-task",
        ):
            code, stdout, stderr = run_dbtool_ee(ee_datadir, verb, _CHUNK_KEY)
            assert code == 0, stderr
            assert "Use --force to execute these changes." in stdout, (verb, stdout)
        # State unchanged after dry runs.
        assert _task(ee_datadir, _CHUNK_KEY)["status"]["name"] == "pending"

        # Abandon the chunk-tagged task → PermanentFailure with the
        # documented reason. The single-key path operates on the opaque
        # key, so no kind flag is needed.
        code, _, stderr = run_dbtool_ee(ee_datadir, "ee-abandon-prover-task", _CHUNK_KEY, "--force")
        assert code == 0, stderr
        record = _task(ee_datadir, _CHUNK_KEY)
        assert record["status"]["name"] == "permanent_failure", record
        assert record["status"]["error"] == "abandoned via dbtool", record
        assert record["kind"] == "chunk", record

        # Reset moves the acct-tagged task back to Pending and clears
        # any retry_after.
        code, _, stderr = run_dbtool_ee(ee_datadir, "ee-reset-prover-task", _ACCT_KEY, "--force")
        assert code == 0, stderr
        record = _task(ee_datadir, _ACCT_KEY)
        assert record["status"]["name"] == "pending", record
        assert "retry_after_secs" not in record, record
        assert record["kind"] == "acct", record

        # Delete the abandoned one; it should drop from the summary.
        code, _, stderr = run_dbtool_ee(ee_datadir, "ee-delete-prover-task", _CHUNK_KEY, "--force")
        assert code == 0, stderr
        after_delete = _summary(ee_datadir)
        assert after_delete["total"] == baseline_total + 1, after_delete
        assert after_delete["permanent_failure"] == baseline_permanent, after_delete

        # Bulk dry run (no --force) prints intent without writing.
        code, stdout, stderr = run_dbtool_ee(
            ee_datadir,
            "ee-abandon-prover-tasks",
            "--all-unfinished",
            "--kind",
            "acct",
        )
        assert code == 0, stderr
        assert "would abandon" in stdout, stdout
        assert "Use --force to execute these changes." in stdout, stdout
        # The acct-tagged task we reset is still Pending after dry-run.
        record = _task(ee_datadir, _ACCT_KEY)
        assert record["status"]["name"] == "pending", record

        # Bulk for-real (kind=acct) flips just our acct task — it must
        # not touch any pre-existing chunk tasks the alpen-client may
        # have left behind.
        code, _, stderr = run_dbtool_ee(
            ee_datadir,
            "ee-abandon-prover-tasks",
            "--all-unfinished",
            "--kind",
            "acct",
            "--force",
        )
        assert code == 0, stderr
        record = _task(ee_datadir, _ACCT_KEY)
        assert record["status"]["name"] == "permanent_failure", record

        return True
