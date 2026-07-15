"""Helpers for invoking strata-dbtool in functional-tests."""

import json
import logging
import subprocess
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)


def extract_json_from_output(output: str) -> dict[str, Any]:
    """Extract and decode first valid JSON object from output text."""
    start = 0
    while True:
        start = output.find("{", start)
        if start == -1:
            raise ValueError(f"No JSON object found in output: {output}")

        depth = 0
        end = -1
        for idx in range(start, len(output)):
            if output[idx] == "{":
                depth += 1
            elif output[idx] == "}":
                depth -= 1
                if depth == 0:
                    end = idx
                    break

        if end == -1:
            raise ValueError(f"Unterminated JSON object in output: {output}")

        candidate = output[start : end + 1]
        try:
            return json.loads(candidate)
        except json.JSONDecodeError:
            start = end + 1
            continue


def run_dbtool_ee(ee_datadir: str, *args: str, timeout: int = 60) -> tuple[int, str, str]:
    """Run strata-dbtool against an alpen-client datadir.

    `ee-*` subcommands open a separate sled at `<datadir>/sled`; the
    dbtool uses the same `-d` flag for both surfaces — callers just point
    it at the alpen-client's `--datadir`.
    """
    cmd = ["strata-dbtool", "-d", ee_datadir, *args]
    logger.info("Running command: %s", " ".join(cmd))
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        cwd=str(Path(ee_datadir).parent),
        timeout=timeout,
    )
    if result.returncode == 0:
        if result.stdout:
            logger.info("Stdout: %s", result.stdout.strip())
    else:
        if result.stderr:
            logger.info("Stderr: %s", result.stderr.strip())
    return result.returncode, result.stdout, result.stderr


def run_dbtool_ee_json(ee_datadir: str, *args: str, timeout: int = 60) -> dict[str, Any]:
    """Run strata-dbtool ee-* command with JSON output and parse response."""
    code, stdout, stderr = run_dbtool_ee(ee_datadir, *args, "-o", "json", timeout=timeout)
    if code != 0:
        raise RuntimeError(
            f"strata-dbtool ee command failed ({' '.join(args)}): {stderr or stdout}"
        )
    return extract_json_from_output(stdout)
