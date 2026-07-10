"""Helpers for invoking alpen-ee-da-tool in functional tests."""

import json
import logging
import os
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from common.services import AlpenClientService, BitcoinService
from tests.alpen_client.ee_da.injection import (
    CraftedEnvelope,
    mine_blocks,
    mine_empty_blocks,
    post_ee_da_envelope,
)

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class DaWindow:
    """Inclusive L1 scan window."""

    start_height: int
    end_height: int


@dataclass(frozen=True)
class ReconstructedDaReport:
    """Tool report plus the inclusive L1 end height used to produce it."""

    end_height: int
    report: dict[str, Any]


# `strata-test-cli post-ee-da-envelope` signs reveal scripts with secret key
# `[42; 32]`. The verifier must be configured with the matching x-only pubkey.
TEST_ENVELOPE_PUBKEY = "5be5e9478209674a96e60f1f037f6176540fd001fa1d64694770c56a7709c42c"

__all__ = [
    "TEST_ENVELOPE_PUBKEY",
    "DaWindow",
    "ReconstructedDaReport",
    "assert_published_inner_root_match",
    "mine_empty_blocks",
    "post_envelope_in_one_block",
    "run_alpen_ee_da_tool",
    "run_alpen_ee_da_tool_json",
    "wait_for_published_inner_root_report",
    "wait_for_reconstructed_real_da_report",
    "wait_for_reconstructed_real_da_report_window",
    "write_reconstruction_config",
    "write_verification_config",
]


def _base_verifier_config_lines(
    bitcoin: BitcoinService,
    sequencer: AlpenClientService,
    *,
    sequencer_pubkey_override: str | None = None,
) -> list[str]:
    """Build config lines shared by reconstruction and verification runs."""
    sequencer_pubkey = sequencer_pubkey_override or sequencer.props["sequencer_pubkey"]
    sequencer_pubkey = sequencer_pubkey.removeprefix("0x")

    return [
        f'bitcoind_url = "http://127.0.0.1:{bitcoin.props["rpc_port"]}/wallet/{bitcoin.props["walletname"]}"',
        f'bitcoind_rpc_user = "{bitcoin.props["rpc_user"]}"',
        f'bitcoind_rpc_password = "{bitcoin.props["rpc_password"]}"',
        f'sequencer_pubkey = "{sequencer_pubkey}"',
        f'ee_params = "{sequencer.props["ee_params_path"]}"',
    ]


def _write_config_file(
    bitcoin: BitcoinService,
    lines: list[str],
    *,
    directory: str | Path | None = None,
) -> Path:
    """Write config lines to a test-local TOML file and return its path."""
    root_dir = Path(directory) if directory is not None else Path(bitcoin.props["datadir"])
    root_dir.mkdir(parents=True, exist_ok=True)
    lines.append("")

    fd, raw_path = tempfile.mkstemp(prefix="alpen-ee-da-tool-", suffix=".toml", dir=root_dir)
    Path(raw_path).write_text(
        "\n".join(lines),
        encoding="utf-8",
    )
    os.close(fd)
    return Path(raw_path)


def write_reconstruction_config(
    bitcoin: BitcoinService,
    sequencer: AlpenClientService,
    *,
    directory: str | Path | None = None,
    sequencer_pubkey_override: str | None = None,
) -> Path:
    """Write config for reconstruction-only runs without published-root lookup."""
    lines = _base_verifier_config_lines(
        bitcoin,
        sequencer,
        sequencer_pubkey_override=sequencer_pubkey_override,
    )
    return _write_config_file(bitcoin, lines, directory=directory)


def write_verification_config(
    bitcoin: BitcoinService,
    sequencer: AlpenClientService,
    *,
    directory: str | Path | None = None,
    sequencer_pubkey_override: str | None = None,
) -> Path:
    """Write config for runs that compare against published Snark account roots."""
    ol_rpc_url = sequencer.props.get("ol_rpc_url")
    assert ol_rpc_url is not None, (
        "alpen-client service must expose ol_rpc_url prop; check factories/alpen_client.py"
    )
    lines = _base_verifier_config_lines(
        bitcoin,
        sequencer,
        sequencer_pubkey_override=sequencer_pubkey_override,
    )
    lines.append(f'ol_rpc_url = "{ol_rpc_url}"')
    return _write_config_file(bitcoin, lines, directory=directory)


def run_alpen_ee_da_tool(
    config_toml_path: str | Path,
    start_height: int,
    end_height: int,
    *,
    expected_root: str | None = None,
    custom_chain: str | None = None,
    snapshot: str | Path | None = None,
    export_snapshot: str | Path | None = None,
    output_format: str | None = None,
    timeout: int = 120,
) -> tuple[int, str, str]:
    """Run alpen-ee-da-tool and return `(return_code, stdout, stderr)`."""
    cmd = [
        "alpen-ee-da-tool",
        "--config",
        str(config_toml_path),
        "--start-height",
        str(start_height),
        "--end-height",
        str(end_height),
    ]
    if output_format is not None:
        cmd.extend(["--output-format", output_format])
    if expected_root is not None:
        cmd.extend(["--expected-root", expected_root])
    if custom_chain is not None:
        cmd.extend(["--custom-chain", custom_chain])
    if snapshot is not None:
        cmd.extend(["--snapshot", str(snapshot)])
    if export_snapshot is not None:
        cmd.extend(["--export-snapshot", str(export_snapshot)])

    logger.info("Running command: %s", " ".join(cmd))
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if result.returncode == 0 and result.stdout:
        logger.info("Stdout: %s", result.stdout.strip())
    elif result.stderr:
        logger.info("Stderr: %s", result.stderr.strip())

    return result.returncode, result.stdout, result.stderr


def extract_json_from_output(output: str) -> dict[str, Any]:
    """Parse the JSON object that spans from the first `{` to the last `}`."""
    start = output.find("{")
    end = output.rfind("}")
    if start == -1 or end == -1 or end < start:
        raise ValueError(f"No JSON object found in output: {output!r}")
    return json.loads(output[start : end + 1])


def run_alpen_ee_da_tool_json(
    config_toml_path: str | Path,
    start_height: int,
    end_height: int,
    *,
    expected_root: str | None = None,
    custom_chain: str | None = None,
    snapshot: str | Path | None = None,
    export_snapshot: str | Path | None = None,
    timeout: int = 120,
) -> tuple[int, dict[str, Any], str]:
    """Run alpen-ee-da-tool with JSON output and parse its report."""
    code, stdout, stderr = run_alpen_ee_da_tool(
        config_toml_path,
        start_height,
        end_height,
        expected_root=expected_root,
        custom_chain=custom_chain,
        snapshot=snapshot,
        export_snapshot=export_snapshot,
        output_format="json",
        timeout=timeout,
    )
    report: dict[str, Any] = {} if code != 0 else extract_json_from_output(stdout)
    return code, report, stderr


def assert_published_inner_root_match(report: dict[str, Any]) -> None:
    """Assert the tool matched reconstructed and published Snark account roots."""
    assert report["reconstructed_inner_state_root"] is not None
    assert report["expected_inner_state_root"] is not None
    assert report["inner_state_root_matches_expected"] is True
    assert report.get("inner_state_root_mismatch_update_seq_no") is None


def wait_for_published_inner_root_report(
    bitcoin: BitcoinService,
    sequencer: AlpenClientService,
    config_toml_path: str | Path,
    start_height: int,
    end_height: int,
    *,
    snapshot: str | Path | None = None,
    export_snapshot: str | Path | None = None,
    poll_attempts: int = 20,
    blocks_per_poll: int = 3,
    timeout: int = 180,
) -> dict[str, Any]:
    """Run the tool until published Snark account root lookup catches up.

    Advances the DA window between attempts so OL can finalize the epoch
    containing the published update manifest record.
    """
    btc_rpc = bitcoin.create_rpc()
    mine_address = btc_rpc.proxy.getnewaddress()
    last_stderr = ""

    for attempt in range(1, poll_attempts + 1):
        code, report, stderr = run_alpen_ee_da_tool_json(
            config_toml_path,
            start_height,
            end_height,
            custom_chain=sequencer.props["chain_spec"],
            snapshot=snapshot,
            export_snapshot=export_snapshot,
            timeout=timeout,
        )
        if code == 0:
            return report

        last_stderr = stderr
        if "No canonical commitment found" in stderr:
            raise AssertionError(
                f"alpen-ee-da-tool hit an unexpected manifest RPC canonical-epoch error: {stderr}"
            )
        if "No Snark account update manifest found" not in stderr:
            raise AssertionError(
                "alpen-ee-da-tool failed for a non-retryable reason while waiting for "
                f"published inner-root lookup: {stderr}"
            )

        logger.info(
            "Attempt %s/%s: published inner root not available yet over fixed range [%s, %s]: %s",
            attempt,
            poll_attempts,
            start_height,
            end_height,
            stderr.strip(),
        )
        sequencer.advance_to_next_da_window(
            additional_blocks=3,
            timeout_per_block=15.0,
            timeout_slack=60,
        )
        btc_rpc.proxy.generatetoaddress(blocks_per_poll, mine_address)
        time.sleep(2)

    raise AssertionError(
        "Published inner-root lookup did not catch up for alpen-ee-da-tool; "
        f"last_stderr={last_stderr}"
    )


def post_envelope_in_one_block(
    bitcoin: BitcoinService,
    *,
    chunks: list[bytes],
    reveal_count: int | None = None,
) -> tuple[DaWindow, CraftedEnvelope]:
    """Post a crafted envelope and mine commit/reveals in the same L1 block."""
    btc_rpc = bitcoin.create_rpc()
    start_height = btc_rpc.proxy.getblockcount() + 1
    envelope = post_ee_da_envelope(
        bitcoin,
        chunks=chunks,
        reveal_count=reveal_count,
        mine_mode="manual",
    )
    mine_blocks(bitcoin, 1)
    end_height = btc_rpc.proxy.getblockcount()
    return DaWindow(start_height, end_height), envelope


def wait_for_reconstructed_real_da_report(
    bitcoin: BitcoinService,
    config_toml_path: str | Path,
    start_height: int,
    *,
    min_last_block_num: int,
    poll_attempts: int = 20,
    blocks_per_poll: int = 3,
    safe_depth: int = 2,
    custom_chain: str = "dev",
    timeout: int = 180,
) -> dict[str, Any]:
    """Mine L1 and poll alpen-ee-da-tool until real DA is reconstructed."""
    return wait_for_reconstructed_real_da_report_window(
        bitcoin,
        config_toml_path,
        start_height,
        min_last_block_num=min_last_block_num,
        poll_attempts=poll_attempts,
        blocks_per_poll=blocks_per_poll,
        safe_depth=safe_depth,
        custom_chain=custom_chain,
        timeout=timeout,
    ).report


def wait_for_reconstructed_real_da_report_window(
    bitcoin: BitcoinService,
    config_toml_path: str | Path,
    start_height: int,
    *,
    min_last_block_num: int,
    min_blob_count: int = 1,
    poll_attempts: int = 20,
    blocks_per_poll: int = 3,
    safe_depth: int = 2,
    custom_chain: str = "dev",
    timeout: int = 180,
) -> ReconstructedDaReport:
    """Mine L1 and return the first tool report that reaches the target DA."""
    btc_rpc = bitcoin.create_rpc()
    mine_address = btc_rpc.proxy.getnewaddress()
    last_stderr = ""
    last_report: dict[str, Any] = {}

    for attempt in range(1, poll_attempts + 1):
        btc_rpc.proxy.generatetoaddress(blocks_per_poll, mine_address)
        time.sleep(2)

        tip_height = btc_rpc.proxy.getblockcount()
        end_height = tip_height - safe_depth
        if end_height < start_height:
            continue

        code, report, stderr = run_alpen_ee_da_tool_json(
            config_toml_path,
            start_height,
            end_height,
            custom_chain=custom_chain,
            timeout=timeout,
        )
        last_stderr = stderr
        last_report = report
        if code != 0:
            if "No canonical commitment found" in stderr:
                raise AssertionError(
                    "alpen-ee-da-tool hit an unexpected manifest RPC canonical-epoch error: "
                    f"{stderr}"
                )
            logger.info(
                "Attempt %s/%s: alpen-ee-da-tool failed over [%s, %s]: %s",
                attempt,
                poll_attempts,
                start_height,
                end_height,
                stderr.strip(),
            )
            continue

        applied_range = report.get("applied_range")
        if (
            int(report.get("blobs_reassembled", 0)) >= min_blob_count
            and applied_range is not None
            and int(applied_range.get("last_block_num", 0)) >= min_last_block_num
        ):
            return ReconstructedDaReport(end_height=end_height, report=report)

        logger.info(
            "Attempt %s/%s: report not past target block %s or blob count %s yet: %s",
            attempt,
            poll_attempts,
            min_last_block_num,
            min_blob_count,
            report,
        )

    raise AssertionError(
        "alpen-ee-da-tool did not reconstruct a real DA blob covering "
        f"EVM block {min_last_block_num}; last_report={last_report}, "
        f"last_stderr={last_stderr}"
    )
