"""Helpers for invoking ee-da-verify in functional tests."""

import json
import logging
import os
import subprocess
import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from common.config.constants import DEV_RECIPIENT_ADDRESS
from common.evm import DEV_ACCOUNT_ADDRESS, send_eth_transfer
from common.services import AlpenClientService, BitcoinService
from tests.alpen_client.ee_da.helpers import scan_for_da_envelopes, trigger_batch_sealing

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class DaWindow:
    """Inclusive L1 scan window that contains DA envelopes."""

    start_height: int
    end_height: int


def write_verifier_config(
    bitcoin: BitcoinService,
    sequencer: AlpenClientService,
    *,
    directory: str | Path | None = None,
    magic_bytes_override: bytes | None = None,
) -> Path:
    """Write a test-local ee-da-verify config TOML and return its path.

    `magic_bytes_override` swaps the verifier-side magic, isolating the
    verifier from the live writer's DA traffic.
    """
    chain_spec = sequencer.props["chain_spec"]
    magic_bytes_bytes = magic_bytes_override or sequencer.props["magic_bytes"]
    magic_bytes = magic_bytes_bytes.decode("ascii")

    root_dir = Path(directory) if directory is not None else Path(bitcoin.props["datadir"])
    root_dir.mkdir(parents=True, exist_ok=True)
    fd, raw_path = tempfile.mkstemp(prefix="ee-da-verify-", suffix=".toml", dir=root_dir)
    Path(raw_path).write_text(
        "\n".join(
            [
                f'bitcoind_url = "http://127.0.0.1:{bitcoin.props["rpc_port"]}"',
                f'bitcoind_rpc_user = "{bitcoin.props["rpc_user"]}"',
                f'bitcoind_rpc_password = "{bitcoin.props["rpc_password"]}"',
                f'magic_bytes = "{magic_bytes}"',
                f'chain_spec = "{chain_spec}"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    os.close(fd)
    return Path(raw_path)


def run_ee_da_verify(
    config_toml_path: str | Path,
    start_height: int,
    end_height: int,
    *,
    expected_root: str | None = None,
    output_format: str | None = None,
    timeout: int = 120,
) -> tuple[int, str, str]:
    """Run ee-da-verify and return `(return_code, stdout, stderr)`."""
    cmd = [
        "ee-da-verify",
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

    logger.info("Running command: %s", " ".join(cmd))
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
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


def run_ee_da_verify_json(
    config_toml_path: str | Path,
    start_height: int,
    end_height: int,
    *,
    expected_root: str | None = None,
    timeout: int = 120,
) -> tuple[int, dict[str, Any], str]:
    """Run ee-da-verify with JSON output and parse its report.

    On non-zero exit, returns an empty report rather than raising a parse
    error — callers should assert on the exit code first.
    """
    code, stdout, stderr = run_ee_da_verify(
        config_toml_path,
        start_height,
        end_height,
        expected_root=expected_root,
        output_format="json",
        timeout=timeout,
    )
    report: dict[str, Any] = {} if code != 0 else extract_json_from_output(stdout)
    return code, report, stderr


def produce_da_window(
    sequencer: AlpenClientService,
    bitcoin: BitcoinService,
    *,
    scan_start_height: int | None = None,
    min_envelopes: int = 1,
    transfer_count: int = 6,
    timeout: int = 240,
) -> DaWindow:
    """Produce DA envelopes and return an inclusive L1 window containing them."""
    if min_envelopes < 1:
        raise ValueError("min_envelopes must be >= 1")

    btc_rpc = bitcoin.create_rpc()
    eth_rpc = sequencer.create_rpc()
    start_height = (
        scan_start_height if scan_start_height is not None else btc_rpc.proxy.getblockcount() + 1
    )

    nonce = int(eth_rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
    for idx in range(transfer_count):
        send_eth_transfer(eth_rpc, nonce + idx, DEV_RECIPIENT_ADDRESS, 10**18)

    # Seal at least one batch and post DA for it.
    trigger_batch_sealing(sequencer, btc_rpc, num_blocks=65)

    mine_address = btc_rpc.proxy.getnewaddress()
    envelopes = bitcoin.mine_until(
        check=lambda: scan_for_da_envelopes(
            btc_rpc,
            start_height,
            btc_rpc.proxy.getblockcount(),
        ),
        predicate=lambda found: len(found) >= min_envelopes,
        error_with=f"Timed out waiting for at least {min_envelopes} DA envelope(s)",
        timeout=timeout,
        step=2.0,
        blocks_per_step=1,
        mine_address=mine_address,
    )

    return DaWindow(
        start_height=min(env.height for env in envelopes),
        end_height=max(env.height for env in envelopes),
    )


def ensure_multi_envelope_window(
    config_toml_path: str | Path,
    sequencer: AlpenClientService,
    bitcoin: BitcoinService,
    *,
    min_envelope_count: int = 2,
    max_attempts: int = 2,
) -> tuple[DaWindow, dict[str, Any]]:
    """
    Produce DA until verifier-observed `envelope_count` reaches `min_envelope_count`.

    `produce_da_window` uses `scan_for_da_envelopes` (Python) to count envelopes
    on L1. The verifier does its own scan and can transiently report a lower
    count (e.g. a reveal confirmed between the two scans, or a walker-level
    rejection the Python scan doesn't replay). A small retry smooths that race;
    any miss is logged so CI flakes surface instead of being silently absorbed.
    """
    if min_envelope_count < 2:
        raise ValueError("min_envelope_count must be >= 2")

    btc_rpc = bitcoin.create_rpc()
    scan_start_height = btc_rpc.proxy.getblockcount() + 1
    last_report: dict[str, Any] | None = None

    for attempt in range(max_attempts):
        window = produce_da_window(
            sequencer,
            bitcoin,
            scan_start_height=scan_start_height,
            min_envelopes=min_envelope_count,
        )
        code, report, stderr = run_ee_da_verify_json(
            config_toml_path,
            window.start_height,
            window.end_height,
            timeout=180,
        )
        if code != 0:
            raise AssertionError(f"ee-da-verify dry-run failed: {stderr}")
        last_report = report
        observed = int(report.get("envelope_count", 0))
        if observed >= min_envelope_count:
            return window, report
        logger.warning(
            "verifier observed %d envelopes, expected >= %d (attempt %d/%d)",
            observed,
            min_envelope_count,
            attempt + 1,
            max_attempts,
        )

    raise AssertionError(
        f"Failed to observe envelope_count >= {min_envelope_count} after {max_attempts} attempts. "
        f"Last report: {last_report}"
    )


def parse_applied_last_block_num(report: dict[str, Any]) -> int:
    """Return `applied_range.last.slot` from a verifier JSON report."""
    return int(report["applied_range"]["last"]["slot"])


def fetch_ee_state_root(sequencer: AlpenClientService, block_num: int) -> str:
    """Fetch the EE state root at `block_num` via `eth_getBlockByNumber`."""
    eth_rpc = sequencer.create_rpc()
    # Second arg is `include_transactions` — headers only is enough.
    block = eth_rpc.eth_getBlockByNumber(hex(block_num), False)
    if block is None:
        raise AssertionError(f"Missing EE block at number {block_num}")
    state_root = block.get("stateRoot")
    if not isinstance(state_root, str):
        raise AssertionError(f"Missing stateRoot for EE block {block_num}: {block}")
    return state_root


def mutate_root_hex(root: str) -> str:
    """Return a deterministic different 32-byte hex root.

    Accepts input with or without a `0x` prefix (Buf32's serde emits bare hex).
    Output is always `0x`-prefixed.
    """
    hex_body = root.removeprefix("0x")
    raw = bytearray.fromhex(hex_body)
    if len(raw) != 32:
        raise ValueError(f"Expected 32-byte root, got {len(raw)} bytes")
    raw[0] ^= 0x01
    return "0x" + raw.hex()


# ---------------------------------------------------------------------------
# Malformed-DA injection helpers
# ---------------------------------------------------------------------------


ZERO_PREV_WTXID_HEX = "00" * 32

# Magic bytes used by injection tests to isolate the verifier from the live
# sequencer's DA traffic (which is tagged with the default `ALPN` magic).
INJECT_MAGIC = b"TEST"


def post_ee_da_envelope(
    bitcoin: BitcoinService,
    *,
    prev_wtxid: str,
    chunks: list[bytes],
    magic_bytes: bytes = INJECT_MAGIC,
    fee_rate: int = 2,
    timeout: int = 60,
) -> list[str]:
    """Invoke `strata-test-cli post-ee-da-envelope` and return reveal wtxids.

    `prev_wtxid` is 64-char hex (no `0x`). Each chunk in `chunks` is the raw
    chunk payload (use `craft_chunk_bytes` to build one).
    """
    if not chunks:
        raise ValueError("chunks must contain at least one entry")

    bitcoind_url = f"http://127.0.0.1:{bitcoin.props['rpc_port']}"
    cmd = [
        "strata-test-cli",
        "post-ee-da-envelope",
        "--bitcoind-url",
        bitcoind_url,
        "--rpc-user",
        bitcoin.props["rpc_user"],
        "--rpc-password",
        bitcoin.props["rpc_password"],
        "--magic-bytes",
        magic_bytes.decode("ascii"),
        "--fee-rate",
        str(fee_rate),
        "--prev-wtxid",
        prev_wtxid,
    ]
    for chunk in chunks:
        cmd.extend(["--chunk-hex", chunk.hex()])

    logger.info("Running: %s", " ".join(cmd))
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError(
            f"post-ee-da-envelope failed ({result.returncode}): {result.stderr or result.stdout}"
        )

    wtxids = [line.strip() for line in result.stdout.splitlines() if line.strip()]
    if not wtxids:
        raise RuntimeError(f"post-ee-da-envelope produced no wtxids: stdout={result.stdout!r}")
    return wtxids


def craft_chunk_bytes(
    blob_hash: bytes,
    chunk_index: int,
    total_chunks: int,
    body: bytes,
) -> bytes:
    """Build a DA chunk payload: `version(1) ++ blob_hash(32)
       ++ index(2 BE) ++ total(2 BE) ++ body`.

    Mirrors `alpen_ee_common::DaChunkHeader` framing; symmetric with
    `tests.alpen_client.ee_da.codec.parse_da_chunk_header`.
    """
    if len(blob_hash) != 32:
        raise ValueError(f"blob_hash must be 32 bytes, got {len(blob_hash)}")
    if not 0 <= chunk_index <= 0xFFFF:
        raise ValueError(f"chunk_index out of u16 range: {chunk_index}")
    if not 0 <= total_chunks <= 0xFFFF:
        raise ValueError(f"total_chunks out of u16 range: {total_chunks}")
    return (
        b"\x00"
        + blob_hash
        + chunk_index.to_bytes(2, "big")
        + total_chunks.to_bytes(2, "big")
        + body
    )


def inject_da_window(
    bitcoin: BitcoinService,
    inject: Callable[[], None],
) -> tuple[int, int]:
    """Bracket `inject` with `getblockcount` reads; return the `(start, end)` L1 window."""
    btc_rpc = bitcoin.create_rpc()
    window_start = btc_rpc.proxy.getblockcount() + 1
    inject()
    window_end = btc_rpc.proxy.getblockcount()
    return window_start, window_end
