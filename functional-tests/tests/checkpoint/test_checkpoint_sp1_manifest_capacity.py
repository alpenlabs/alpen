"""Manual SP1 checkpoint capacity proof test.

This test is intentionally opt-in. It does not start Strata/Bitcoin services; it
drives the checkpoint SP1 guest through `strata-provers-perf` with the synthetic
capacity input used for `MAX_SEALING_MANIFEST_COUNT` analysis.

Set `RUN_SP1_CHECKPOINT_CAPACITY_FN_TEST=1` to run it. By default it proves the
largest candidate (`2048` manifests); override with
`CHECKPOINT_CAPACITY_MANIFEST_COUNTS=1024` or a comma-separated list.
Set `CHECKPOINT_CAPACITY_OL_LOG_TARGET=16128` to run the same manifest count with
near-hard-cap OL log pressure.
Set `CHECKPOINT_CAPACITY_ASM_LOGS_PER_MANIFEST=128` to run the same manifest
count with near-cap deposit-log pressure in each ASM manifest.
Set `SP1_CHECKPOINT_CAPACITY_RUNS=3` to repeat the same proof workload for
stability.
"""

import logging
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import flexitest

from common.base_test import BaseTest

logger = logging.getLogger(__name__)

RUN_ENV = "RUN_SP1_CHECKPOINT_CAPACITY_FN_TEST"
MANIFEST_COUNTS_ENV = "CHECKPOINT_CAPACITY_MANIFEST_COUNTS"
ASM_LOGS_PER_MANIFEST_ENV = "CHECKPOINT_CAPACITY_ASM_LOGS_PER_MANIFEST"
OL_LOG_TARGET_ENV = "CHECKPOINT_CAPACITY_OL_LOG_TARGET"
TIMEOUT_ENV = "SP1_CHECKPOINT_CAPACITY_TIMEOUT_SECS"
RUNS_ENV = "SP1_CHECKPOINT_CAPACITY_RUNS"


class NoServicesEnv(flexitest.EnvConfig):
    """Functional-test env with no long-running services."""

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        return flexitest.LiveEnv({})


@flexitest.register
class TestCheckpointSp1ManifestCapacity(BaseTest):
    """Prove the checkpoint capacity input with SP1 in a manual functional test."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(NoServicesEnv())

    def main(self, ctx) -> bool:  # type: ignore[override]
        if os.getenv(RUN_ENV) != "1":
            logger.info("skipping manual SP1 capacity test; set %s=1 to run it", RUN_ENV)
            return True

        repo_root = Path(__file__).resolve().parents[3]
        manifest_counts = os.getenv(MANIFEST_COUNTS_ENV, "2048")
        asm_logs_per_manifest = os.getenv(ASM_LOGS_PER_MANIFEST_ENV)
        ol_log_target = os.getenv(OL_LOG_TARGET_ENV)
        timeout = int(os.getenv(TIMEOUT_ENV, "21600"))
        runs = int(os.getenv(RUNS_ENV, "1"))
        assert runs > 0, f"{RUNS_ENV} must be positive"

        env = os.environ.copy()
        env[MANIFEST_COUNTS_ENV] = manifest_counts
        if asm_logs_per_manifest is not None:
            env[ASM_LOGS_PER_MANIFEST_ENV] = asm_logs_per_manifest
        if ol_log_target is not None:
            env[OL_LOG_TARGET_ENV] = ol_log_target
        env.pop("ZKVM_MOCK", None)

        build_cmd = [
            "cargo",
            "build",
            "--release",
            "-p",
            "strata-provers-perf",
        ]
        prover_binary = repo_root / "target" / "release" / "strata-provers-perf"
        cmd = [
            str(prover_binary),
            "--prove",
            "--programs",
            "checkpoint-capacity",
        ]

        logger.info(
            "building strata-provers-perf before SP1 capacity proof: timeout=%ss",
            timeout,
        )
        build_result = subprocess.run(
            build_cmd,
            cwd=repo_root,
            env=env,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        if build_result.stdout:
            logger.info("cargo build stdout:\n%s", build_result.stdout)
        if build_result.stderr:
            logger.info("cargo build stderr:\n%s", build_result.stderr)
        assert build_result.returncode == 0, (
            f"failed to build strata-provers-perf (exit {build_result.returncode})"
        )

        time_prefix = []
        if shutil.which("/usr/bin/time"):
            time_prefix = ["/usr/bin/time", "-l" if sys.platform == "darwin" else "-v"]

        logger.info(
            "running real SP1 checkpoint capacity proof: manifest_counts=%s "
            "asm_logs_per_manifest=%s ol_log_target=%s timeout=%ss sp1_prover=%s runs=%s",
            manifest_counts,
            asm_logs_per_manifest or "<default>",
            ol_log_target or "<default>",
            timeout,
            env.get("SP1_PROVER", "<sp1 default>"),
            runs,
        )
        for run_idx in range(1, runs + 1):
            logger.info("starting SP1 checkpoint capacity proof run %s/%s", run_idx, runs)
            started_at = time.monotonic()
            result = subprocess.run(
                [*time_prefix, *cmd],
                cwd=repo_root,
                env=env,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
            elapsed_secs = time.monotonic() - started_at

            if result.stdout:
                logger.info(
                    "strata-provers-perf stdout for run %s/%s:\n%s",
                    run_idx,
                    runs,
                    result.stdout,
                )
            if result.stderr:
                logger.info(
                    "strata-provers-perf stderr for run %s/%s:\n%s",
                    run_idx,
                    runs,
                    result.stderr,
                )
            logger.info(
                "SP1 checkpoint capacity proof run %s/%s wall-clock: %.3fs",
                run_idx,
                runs,
                elapsed_secs,
            )

            assert result.returncode == 0, (
                "SP1 checkpoint capacity proof failed "
                f"(exit {result.returncode}; manifest_counts={manifest_counts}; "
                f"asm_logs_per_manifest={asm_logs_per_manifest or '<default>'}; "
                f"ol_log_target={ol_log_target or '<default>'}; "
                f"run={run_idx}/{runs})"
            )

            for count in [part.strip() for part in manifest_counts.split(",") if part.strip()]:
                assert f"Checkpoint-{count}" in result.stdout, (
                    f"missing proof result row for Checkpoint-{count} (run={run_idx}/{runs})"
                )

            assert "*SP1 Proof Results*" in result.stdout
        return True
