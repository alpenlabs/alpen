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

from common.services import AlpenClientService, BitcoinService

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class DaWindow:
    """Inclusive L1 scan window."""

    start_height: int
    end_height: int


# Magic bytes used by injection tests to isolate verifier input from the live
# sequencer DA writer while this stack tests the new verifier-side parser.
INJECT_MAGIC = b"TEST"


def write_verifier_config(
    bitcoin: BitcoinService,
    sequencer: AlpenClientService,
    *,
    directory: str | Path | None = None,
    magic_bytes_override: bytes | None = None,
) -> Path:
    """Write a test-local ee-da-verify config TOML and return its path."""
    chain_spec = sequencer.props["chain_spec"]
    magic_bytes_bytes = magic_bytes_override or sequencer.props["magic_bytes"]
    magic_bytes = magic_bytes_bytes.decode("ascii")
    sequencer_pubkey = sequencer.props["sequencer_pubkey"].removeprefix("0x")

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
                f'sequencer_pubkey = "{sequencer_pubkey}"',
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


def run_ee_da_verify_json(
    config_toml_path: str | Path,
    start_height: int,
    end_height: int,
    *,
    expected_root: str | None = None,
    timeout: int = 120,
) -> tuple[int, dict[str, Any], str]:
    """Run ee-da-verify with JSON output and parse its report."""
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


def mutate_root_hex(root: str) -> str:
    """Return a deterministic different 32-byte hex root."""
    hex_body = root.removeprefix("0x")
    raw = bytearray.fromhex(hex_body)
    if len(raw) != 32:
        raise ValueError(f"Expected 32-byte root, got {len(raw)} bytes")
    raw[0] ^= 0x01
    return "0x" + raw.hex()


def post_ee_da_envelope(
    bitcoin: BitcoinService,
    sequencer: AlpenClientService,
    *,
    chunks: list[bytes],
    magic_bytes: bytes = INJECT_MAGIC,
    malformed: str = "none",
    timeout: int = 60,
) -> dict[str, list[str] | str]:
    """Invoke `strata-test-cli post-ee-da-envelope` for injected DA."""
    if not chunks and malformed != "missing-reveal-slots":
        raise ValueError("chunks must contain at least one entry")

    sequencer_secret_key = sequencer.props["sequencer_privkey"]
    if sequencer_secret_key is None:
        raise ValueError("sequencer private key is required for injected DA")

    cmd = [
        "strata-test-cli",
        "post-ee-da-envelope",
        "--bitcoind-url",
        f"http://127.0.0.1:{bitcoin.props['rpc_port']}",
        "--rpc-user",
        bitcoin.props["rpc_user"],
        "--rpc-password",
        bitcoin.props["rpc_password"],
        "--magic-bytes",
        magic_bytes.decode("ascii"),
        "--sequencer-secret-key",
        sequencer_secret_key,
        "--malformed",
        malformed,
    ]
    for chunk in chunks:
        cmd.extend(["--chunk-hex", chunk.hex()])

    logger.info("Running: %s", " ".join(cmd))
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError(
            f"post-ee-da-envelope failed ({result.returncode}): {result.stderr or result.stdout}"
        )

    parsed: dict[str, list[str] | str] = {"reveal_txid": []}
    for line in result.stdout.splitlines():
        key, _, value = line.strip().partition("=")
        if key == "commit_txid":
            parsed[key] = value
        elif key == "reveal_txid":
            assert isinstance(parsed["reveal_txid"], list)
            parsed["reveal_txid"].append(value)
    if "commit_txid" not in parsed:
        raise RuntimeError(f"post-ee-da-envelope produced no commit txid: {result.stdout!r}")
    return parsed


def inject_da_window(bitcoin: BitcoinService, inject: Callable[[], None]) -> DaWindow:
    """Bracket `inject` with `getblockcount` reads and return the L1 window."""
    btc_rpc = bitcoin.create_rpc()
    window_start = btc_rpc.proxy.getblockcount() + 1
    inject()
    window_end = btc_rpc.proxy.getblockcount()
    return DaWindow(window_start, window_end)


def craft_single_chunk_blob(body: bytes) -> list[bytes]:
    """Wrap raw encoded blob bytes in one reveal chunk."""
    return [body]


def encode_synthetic_empty_da_blob(update_seq_no: int = 0, block_num: int = 1) -> bytes:
    """Encode a minimal DA blob with an empty state diff."""
    if update_seq_no < 0:
        raise ValueError(f"update_seq_no must be non-negative, got {update_seq_no}")
    if block_num < 0:
        raise ValueError(f"block_num must be non-negative, got {block_num}")

    evm_header = b"".join(
        value.to_bytes(8, "big")
        for value in [block_num, 1_700_000_000 + block_num, 1_000_000_000, 0, 30_000_000]
    )
    empty_state_diff = b"\x00\x00\x00\x00" * 3
    return update_seq_no.to_bytes(8, "big") + evm_header + empty_state_diff


def synthetic_empty_da_blob_chunks(
    _sequencer: AlpenClientService,
    block_num: int = 1,
) -> list[bytes]:
    """Build chunks for an empty synthetic DA blob using the current blob schema."""
    return craft_single_chunk_blob(encode_synthetic_empty_da_blob(block_num=block_num))


def post_synthetic_da_window(bitcoin: BitcoinService, sequencer: AlpenClientService) -> DaWindow:
    """Post one valid synthetic DA blob and return its L1 window."""
    chunks = synthetic_empty_da_blob_chunks(sequencer)
    return inject_da_window(
        bitcoin,
        lambda: post_ee_da_envelope(bitcoin, sequencer, chunks=chunks),
    )
