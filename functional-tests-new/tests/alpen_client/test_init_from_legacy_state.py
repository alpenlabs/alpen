"""Test that imports EE state dumped from the legacy chain via `reth init-state`.

Reads artifacts produced by `functional-tests/tests/el_dump_state.py`:
  - state_dump.jsonl  — JSONL state dump
  - header.rlp        — RLP-encoded block header
  - dump_info.json    — metadata (block number, state root, balances)

Then runs `reth init-state --without-evm` on a fresh datadir, starts the reth
node, and verifies that all accounts match the dumped state.

Prerequisites:
  - Run `cd functional-tests && ./run_test.sh -t el_dump_state` first to produce
    the artifacts in `functional-tests/_state_dump_artifacts/`.
  - `reth` binary must be on PATH.
"""

import json
import logging
import shutil
import socket
import subprocess
from pathlib import Path

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.rpc import JsonRpcClient
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

# Path to artifacts produced by functional-tests/tests/el_dump_state.py
LEGACY_ARTIFACTS_DIR = (
    Path(__file__).resolve().parents[3] / "functional-tests" / "_state_dump_artifacts"
)

# Chain spec — must match what the legacy chain used
DEV_CHAIN_SPEC = "crates/reth/chainspec/src/res/alpen-dev-chain.json"


def normalize_hex_address(address: str) -> str:
    return address.lower() if address.startswith("0x") else f"0x{address.lower()}"


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


@flexitest.register
class TestInitFromLegacyState(BaseTest):
    """Import legacy chain state into fresh reth and verify all accounts."""

    def __init__(self, ctx: flexitest.InitContext):
        # No env needed — this test doesn't start any services
        ctx.set_env("basic")

    def main(self, ctx):
        # --- Phase 1: Load artifacts ---
        assert LEGACY_ARTIFACTS_DIR.is_dir(), (
            f"Legacy artifacts not found at {LEGACY_ARTIFACTS_DIR}. "
            "Run 'cd functional-tests && ./run_test.sh -t el_dump_state' first."
        )

        dump_info_path = LEGACY_ARTIFACTS_DIR / "dump_info.json"
        jsonl_path = LEGACY_ARTIFACTS_DIR / "state_dump.jsonl"
        header_path = LEGACY_ARTIFACTS_DIR / "header.rlp"

        assert dump_info_path.exists(), f"Missing: {dump_info_path}"
        assert jsonl_path.exists(), f"Missing: {jsonl_path}"
        assert header_path.exists(), f"Missing: {header_path}"

        with open(dump_info_path) as f:
            dump_info = json.load(f)

        block_number = dump_info["block_number"]
        state_root = dump_info["state_root"]
        account_count = dump_info["account_count"]
        logger.info(
            "Loaded legacy dump: block %d, %d accounts, root %s",
            block_number,
            account_count,
            state_root,
        )

        # Parse accounts from JSONL for later verification
        expected_accounts = {}
        with open(jsonl_path) as f:
            lines = f.readlines()
        for line in lines[1:]:  # skip root line
            entry = json.loads(line)
            addr = normalize_hex_address(entry["address"])
            expected_accounts[addr] = entry
        logger.info("Parsed %d accounts from JSONL", len(expected_accounts))

        # --- Phase 2: Run reth init-state ---
        reth_bin = shutil.which("reth")
        assert reth_bin is not None, (
            "'reth' binary not found on PATH. Install with: "
            "cargo install reth --git https://github.com/paradigmxyz/reth --tag v1.9.1"
        )

        repo_root = Path(__file__).resolve().parents[3]
        chain_spec_path = repo_root / DEV_CHAIN_SPEC
        assert chain_spec_path.exists(), f"Chain spec not found: {chain_spec_path}"

        # Use a service's datadir parent as the working directory for our import
        strata_svc = self.get_service(ServiceType.Strata)
        work_dir = Path(strata_svc.props["datadir"]).parent / "reth_imported"
        fresh_datadir = work_dir
        if fresh_datadir.exists():
            shutil.rmtree(fresh_datadir)
        fresh_datadir.mkdir(parents=True)
        log_dir = fresh_datadir / "logs"
        log_dir.mkdir(parents=True, exist_ok=True)

        cmd = [
            reth_bin,
            "init-state",
            str(jsonl_path),
            "--without-evm",
            "--header",
            str(header_path),
            "--chain",
            str(chain_spec_path),
            "--datadir",
            str(fresh_datadir),
            "--log.file.directory",
            str(log_dir),
        ]
        logger.info("Running: %s", " ".join(cmd))

        result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        if result.returncode != 0:
            logger.error("stdout: %s", result.stdout)
            logger.error("stderr: %s", result.stderr)
            raise AssertionError(
                f"reth init-state failed (exit {result.returncode}): {result.stderr}"
            )
        logger.info("reth init-state succeeded")

        # --- Phase 3: Validate DB artifacts exist ---
        db_dir = fresh_datadir / "db"
        assert db_dir.is_dir(), f"Missing db dir: {db_dir}"
        mdbx_path = db_dir / "mdbx.dat"
        assert mdbx_path.exists() and mdbx_path.stat().st_size > 0, (
            f"Empty or missing MDBX: {mdbx_path}"
        )
        logger.info("DB artifacts validated")

        # --- Phase 4: Start reth node and verify state via RPC ---
        http_port = find_free_port()
        authrpc_port = find_free_port()
        p2p_port = find_free_port()
        node_log_path = fresh_datadir / "reth_node.log"

        node_cmd = [
            reth_bin,
            "node",
            "--chain",
            str(chain_spec_path),
            "--datadir",
            str(fresh_datadir),
            "--http",
            "--http.addr",
            "127.0.0.1",
            "--http.port",
            str(http_port),
            "--authrpc.port",
            str(authrpc_port),
            "--port",
            str(p2p_port),
            "--disable-discovery",
            "--log.file.directory",
            str(log_dir),
        ]
        logger.info("Starting reth node: %s", " ".join(node_cmd))

        with open(node_log_path, "w") as node_log:
            process = subprocess.Popen(node_cmd, stdout=node_log, stderr=node_log, text=True)

        rpc = JsonRpcClient(f"http://127.0.0.1:{http_port}", name="reth_verify", timeout=5)

        try:
            # Wait for RPC to come up and verify tip
            def wait_for_rpc():
                if process.poll() is not None:
                    raise AssertionError(
                        f"reth exited early (code {process.returncode}); see {node_log_path}"
                    )
                return int(rpc.eth_blockNumber(), 16)

            tip = wait_until_with_value(
                wait_for_rpc,
                lambda n: n >= 0,
                error_with="Timed out waiting for reth RPC",
                timeout=30,
                step=0.5,
            )
            assert tip == block_number, f"Tip mismatch: expected {block_number}, got {tip}"
            logger.info("Reth tip at block %d — matches dump", tip)

            # Verify state root
            latest_block = rpc.eth_getBlockByNumber("latest", False)
            assert latest_block["stateRoot"].lower() == state_root.lower(), (
                f"State root mismatch: expected {state_root}, got {latest_block['stateRoot']}"
            )
            logger.info("State root matches: %s", state_root)

            # Verify each account
            for addr, expected in expected_accounts.items():
                rpc_balance = int(rpc.eth_getBalance(addr, "latest"), 16)
                expected_balance = int(expected.get("balance", "0x0"), 16)
                assert rpc_balance == expected_balance, (
                    f"Balance mismatch for {addr}: expected {expected_balance}, got {rpc_balance}"
                )

                rpc_nonce = int(rpc.eth_getTransactionCount(addr, "latest"), 16)
                expected_nonce = int(expected.get("nonce", 0))
                assert rpc_nonce == expected_nonce, (
                    f"Nonce mismatch for {addr}: expected {expected_nonce}, got {rpc_nonce}"
                )

                rpc_code = rpc.eth_getCode(addr, "latest")
                expected_code = expected.get("code", "0x")
                if not expected_code.startswith("0x"):
                    expected_code = f"0x{expected_code}"
                assert rpc_code.lower() == expected_code.lower(), (
                    f"Code mismatch for {addr}: expected {expected_code}, got {rpc_code}"
                )

            logger.info("All %d accounts verified", len(expected_accounts))

        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

        logger.info("Legacy state import and verification complete")
        return True
