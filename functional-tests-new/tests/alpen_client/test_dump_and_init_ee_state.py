"""Test that dumps EE state and validates it can be imported via `reth init-state`.

Starts the full EE+OL stack, creates EVM state, waits for:
  - EE blocks produced
  - A batch submitted as an update to OL
  - The OL epoch containing the update is finalized

Then dumps the complete EE state and block header, validates the artifacts,
and runs `reth init-state` to verify they are consumable.

Outputs are written to <test_datadir>/state_dump/:
  - state_dump.jsonl  — JSONL state dump (format expected by `reth init-state`)
  - header.rlp        — RLP-encoded block header at dump point
  - dump_info.json    — metadata (block number, state root, block hash, balances)
"""

import json
import logging
import shutil
import socket
import subprocess
from pathlib import Path

import flexitest

from common.accounts import get_dev_account
from common.base_test import BaseTest
from common.config.constants import ALPEN_ACCOUNT_ID, ServiceType
from common.evm_utils import create_funded_account, get_balance, wait_for_receipt
from common.rpc import JsonRpcClient
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

FUND_AMOUNT_WEI = 10 * 10**18
TRANSFER_AMOUNT_WEI = 10**18

# Chain spec for dev chain — must match what alpen-client uses
DEV_CHAIN_SPEC = "crates/reth/chainspec/src/res/alpen-dev-chain.json"


def normalize_hex_address(address: str) -> str:
    """Normalize an address string to lowercase 0x-prefixed hex."""
    return address.lower() if address.startswith("0x") else f"0x{address.lower()}"


def find_free_port() -> int:
    """Allocate an available localhost TCP port."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def write_state_dump_jsonl(state_dump: dict, path: Path) -> int:
    """Convert strataee_dumpState response to JSONL format for reth init-state.

    Returns the number of accounts written.
    """
    count = 0
    with open(path, "w") as f:
        f.write(json.dumps({"root": state_dump["root"]}) + "\n")

        for address, account_data in state_dump["accounts"].items():
            entry = {
                "address": address if address.startswith("0x") else f"0x{address}",
                "balance": account_data.get("balance", "0x0"),
                "nonce": account_data.get("nonce", 0),
                "code": account_data.get("code", "0x"),
                "storage": account_data.get("storage", {}),
            }
            f.write(json.dumps(entry) + "\n")
            count += 1

    return count


def validate_jsonl_format(path: Path) -> int:
    """Validate JSONL file is well-formed for reth init-state.

    Returns the number of account lines.
    """
    with open(path) as f:
        lines = f.readlines()

    assert len(lines) >= 1, "JSONL file is empty"

    # First line must have root
    root_line = json.loads(lines[0])
    assert "root" in root_line, f"First line missing 'root': {root_line}"
    assert root_line["root"].startswith("0x"), f"Root not hex: {root_line['root']}"

    # Remaining lines are accounts
    account_count = 0
    for i, line in enumerate(lines[1:], start=2):
        entry = json.loads(line)
        assert "address" in entry, f"Line {i} missing 'address'"
        assert "balance" in entry, f"Line {i} missing 'balance'"
        assert "nonce" in entry, f"Line {i} missing 'nonce'"
        account_count += 1

    return account_count


def wait_for_update_in_ol(strata_seq: StrataService, strata_rpc, btc_rpc, min_epoch=1, timeout=120):
    """Mine L1 blocks and wait until alpen-client submits an update to OL.

    Only considers epochs >= min_epoch to avoid matching stale updates.
    Returns the epoch number that contains the update.
    """
    mine_address = btc_rpc.proxy.getnewaddress()

    def poll():
        btc_rpc.proxy.generatetoaddress(2, mine_address)
        status = strata_seq.get_sync_status(strata_rpc)
        tip_epoch = status["tip"]["epoch"]

        for ep in range(min_epoch, tip_epoch + 1):
            summary = strata_rpc.strata_getAccountEpochSummary(ALPEN_ACCOUNT_ID, ep)
            if summary and summary.get("update_input") is not None:
                return ep
        return None

    return wait_until_with_value(
        poll,
        lambda ep: ep is not None,
        error_with="Timed out waiting for alpen update in OL",
        timeout=timeout,
    )


def wait_for_finalized_epoch(
    strata_seq: StrataService, strata_rpc, btc_rpc, target_epoch, timeout=120
):
    """Mine L1 blocks until the target epoch is finalized."""
    mine_address = btc_rpc.proxy.getnewaddress()

    def poll():
        btc_rpc.proxy.generatetoaddress(1, mine_address)
        status = strata_seq.get_sync_status(strata_rpc)
        finalized = status.get("finalized")
        if finalized and finalized.get("epoch", -1) >= target_epoch:
            return finalized
        return None

    return wait_until_with_value(
        poll,
        lambda f: f is not None,
        error_with=f"Timed out waiting for epoch {target_epoch} to be finalized",
        timeout=timeout,
    )


def run_reth_init_state(
    jsonl_path: Path,
    header_path: Path,
    chain_spec_path: Path,
    fresh_datadir: Path,
):
    """Run `reth init-state` on a clean datadir.

    Raises AssertionError if `reth` binary is not found or if the import fails.
    """
    reth_bin = shutil.which("reth")
    assert reth_bin is not None, (
        "'reth' binary not found on PATH. Install with: "
        "cargo install reth --git https://github.com/paradigmxyz/reth --tag v1.9.1"
    )

    # Ensure a clean datadir — reth init-state --without-evm expects empty state
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
        logger.error("reth init-state failed (exit %d)", result.returncode)
        logger.error("stdout: %s", result.stdout)
        logger.error("stderr: %s", result.stderr)
        raise AssertionError(f"reth init-state failed: {result.stderr or result.stdout}")

    logger.info("reth init-state succeeded")


def validate_imported_datadir(fresh_datadir: Path):
    """Validate that `reth init-state` created expected DB artifacts."""
    db_dir = fresh_datadir / "db"
    static_files_dir = fresh_datadir / "static_files"
    mdbx_path = db_dir / "mdbx.dat"
    version_path = db_dir / "database.version"

    assert db_dir.is_dir(), f"Missing db dir: {db_dir}"
    assert static_files_dir.is_dir(), f"Missing static files dir: {static_files_dir}"
    assert mdbx_path.exists(), f"Missing MDBX file: {mdbx_path}"
    assert mdbx_path.stat().st_size > 0, f"MDBX file is empty: {mdbx_path}"
    assert version_path.exists(), f"Missing database version file: {version_path}"


def verify_imported_state_via_rpc(
    fresh_datadir: Path,
    chain_spec_path: Path,
    expected_block_number: int,
    expected_state_root: str,
    expected_accounts: dict[str, dict],
    addresses_to_check: list[str],
):
    """Start `reth node` on imported datadir and validate state over JSON-RPC."""
    reth_bin = shutil.which("reth")
    assert reth_bin is not None, "'reth' binary not found on PATH"

    http_port = find_free_port()
    authrpc_port = find_free_port()
    p2p_port = find_free_port()
    log_dir = fresh_datadir / "logs"
    log_dir.mkdir(parents=True, exist_ok=True)
    node_log_path = fresh_datadir / "reth_node_verify.log"

    cmd = [
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
    logger.info("Running: %s", " ".join(cmd))

    with open(node_log_path, "w") as node_log:
        process = subprocess.Popen(cmd, stdout=node_log, stderr=node_log, text=True)

    rpc = JsonRpcClient(f"http://127.0.0.1:{http_port}", name="reth_init_verify", timeout=5)

    try:

        def wait_for_rpc_tip():
            if process.poll() is not None:
                raise AssertionError(
                    f"reth node exited early with code {process.returncode}; "
                    f"see log: {node_log_path}"
                )
            return int(rpc.eth_blockNumber(), 16)

        imported_tip = wait_until_with_value(
            wait_for_rpc_tip,
            lambda n: n >= 0,
            error_with="Timed out waiting for reth RPC after init-state import",
            timeout=30,
            step=0.5,
        )
        assert imported_tip == expected_block_number, (
            f"Imported tip mismatch: expected {expected_block_number}, got {imported_tip}"
        )

        latest_block = rpc.eth_getBlockByNumber("latest", False)
        assert latest_block is not None, "eth_getBlockByNumber(latest) returned null"
        latest_number = int(latest_block["number"], 16)
        assert latest_number == expected_block_number, (
            f"Latest block number mismatch: expected {expected_block_number}, got {latest_number}"
        )
        assert latest_block["stateRoot"].lower() == expected_state_root.lower(), (
            "Imported state root mismatch: "
            f"expected {expected_state_root}, got {latest_block['stateRoot']}"
        )

        for address in addresses_to_check:
            normalized = normalize_hex_address(address)
            expected = expected_accounts.get(normalized)
            assert expected is not None, f"Expected account missing from dump: {address}"

            rpc_balance = int(rpc.eth_getBalance(normalized, "latest"), 16)
            expected_balance = int(expected.get("balance", "0x0"), 16)
            assert rpc_balance == expected_balance, (
                f"Balance mismatch for {normalized}: expected {expected_balance}, got {rpc_balance}"
            )

            rpc_nonce = int(rpc.eth_getTransactionCount(normalized, "latest"), 16)
            expected_nonce = int(expected.get("nonce", 0))
            assert rpc_nonce == expected_nonce, (
                f"Nonce mismatch for {normalized}: expected {expected_nonce}, got {rpc_nonce}"
            )

            rpc_code = rpc.eth_getCode(normalized, "latest")
            expected_code = expected.get("code", "0x")
            expected_code_norm = (
                expected_code if expected_code.startswith("0x") else f"0x{expected_code}"
            )
            assert rpc_code.lower() == expected_code_norm.lower(), (
                f"Code mismatch for {normalized}: expected {expected_code_norm}, got {rpc_code}"
            )
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


@flexitest.register
class TestDumpAndInitEeState(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("el_ol")

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)

        # --- Wait for services ---
        logger.info("Waiting for services...")
        strata_rpc = strata_seq.wait_for_rpc_ready(timeout=20)
        btc_rpc = bitcoin.create_rpc()
        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,
            rpc=strata_rpc,
            timeout=20,
        )
        alpen_seq.wait_for_block(3, timeout=60)

        # Record the current OL epoch so we only look for updates after this point
        pre_tx_status = strata_seq.get_sync_status(strata_rpc)
        pre_tx_epoch = pre_tx_status["tip"]["epoch"]
        logger.info("Pre-transaction OL epoch: %s", pre_tx_epoch)

        # --- Phase 1: Create EVM state ---
        logger.info("Creating EVM state...")
        rpc = alpen_seq.create_rpc()
        dev_account = get_dev_account(rpc)

        funded = create_funded_account(rpc, dev_account, FUND_AMOUNT_WEI)
        logger.info("Funded account %s with %s wei", funded.address, FUND_AMOUNT_WEI)

        recipient = "0x000000000000000000000000000000000000dEaD"
        gas_price = int(rpc.eth_gasPrice(), 16)
        raw_tx = funded.sign_transfer(
            to=recipient,
            value=TRANSFER_AMOUNT_WEI,
            gas_price=gas_price,
            gas=21000,
        )
        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", f"Transfer failed: {receipt}"
        logger.info("Transfer mined in block %s", receipt["blockNumber"])

        # --- Phase 2: Wait for batch → OL update → finalization ---
        min_update_epoch = pre_tx_epoch + 1
        logger.info("Waiting for alpen update in OL (epoch >= %s)...", min_update_epoch)
        update_epoch = wait_for_update_in_ol(
            strata_seq, strata_rpc, btc_rpc, min_epoch=min_update_epoch
        )
        logger.info("Alpen update appeared in OL epoch %s", update_epoch)

        logger.info("Waiting for epoch %s to be finalized...", update_epoch)
        finalized = wait_for_finalized_epoch(strata_seq, strata_rpc, btc_rpc, update_epoch)
        logger.info(
            "Epoch %s finalized — last_slot: %s, last_blkid: %s",
            update_epoch,
            finalized["last_slot"],
            finalized["last_blkid"],
        )

        # Wait a couple more EE blocks after finalization
        alpen_seq.wait_for_additional_blocks(2)

        # --- Phase 3: Record balances ---
        dev_balance = get_balance(rpc, dev_account.address)
        funded_balance = get_balance(rpc, funded.address)
        dead_balance = get_balance(rpc, recipient)
        logger.info(
            "Balances — dev: %s, funded: %s, dead: %s",
            dev_balance,
            funded_balance,
            dead_balance,
        )

        # --- Phase 4: Dump state via custom RPC ---
        logger.info("Dumping state at latest block...")

        state_dump = rpc.strataee_dumpState(None)
        assert state_dump is not None, "strataee_dumpState returned null"
        account_count = len(state_dump["accounts"])
        state_root = state_dump["root"]
        block_num_int = state_dump["block_number"]
        logger.info(
            "State dump: block %d, %d accounts, root: %s",
            block_num_int,
            account_count,
            state_root,
        )

        # Verify known accounts are in the dump
        dump_addresses = {addr.lower() for addr in state_dump["accounts"]}
        assert dev_account.address.lower() in dump_addresses, (
            f"Dev account {dev_account.address} not found in dump"
        )
        assert funded.address.lower() in dump_addresses, (
            f"Funded account {funded.address} not found in dump"
        )

        # Get block header and verify state root consistency
        block_hex = hex(block_num_int)
        block = rpc.eth_getBlockByNumber(block_hex, False)
        block_hash = block["hash"]
        logger.info("Block %d — hash: %s", block_num_int, block_hash)

        assert block["stateRoot"] == state_root, (
            f"State root mismatch: dump says {state_root}, block header says {block['stateRoot']}"
        )

        raw_header = rpc.debug_getRawHeader(block_hex)
        logger.info("Raw header: %d bytes", (len(raw_header) - 2) // 2)

        # --- Phase 5: Write artifacts ---
        output_dir = Path(alpen_seq.props["datadir"]).parent / "state_dump"
        output_dir.mkdir(parents=True, exist_ok=True)

        # Write JSONL state dump
        jsonl_path = output_dir / "state_dump.jsonl"
        written_count = write_state_dump_jsonl(state_dump, jsonl_path)
        logger.info("Wrote %d accounts to %s", written_count, jsonl_path)

        # Write RLP header
        header_path = output_dir / "header.rlp"
        header_hex = raw_header[2:] if raw_header.startswith("0x") else raw_header
        header_path.write_text(header_hex)
        logger.info("Wrote header to %s", header_path)

        # Write metadata
        dump_info = {
            "block_number": block_num_int,
            "block_hash": block_hash,
            "state_root": state_root,
            "account_count": account_count,
            "finalized_epoch": update_epoch,
            "finalized_info": finalized,
            "balances": {
                dev_account.address: str(dev_balance),
                funded.address: str(funded_balance),
                recipient: str(dead_balance),
            },
            "artifacts": {
                "state_dump": str(jsonl_path),
                "header": str(header_path),
            },
        }
        info_path = output_dir / "dump_info.json"
        with open(info_path, "w") as f:
            json.dump(dump_info, f, indent=2)
        logger.info("Wrote dump info to %s", info_path)

        # --- Phase 6: Validate JSONL format ---
        logger.info("Validating JSONL format...")
        validated_count = validate_jsonl_format(jsonl_path)
        assert validated_count == written_count, (
            f"JSONL validation count mismatch: wrote {written_count}, validated {validated_count}"
        )
        logger.info("JSONL format valid: %d accounts", validated_count)

        # --- Phase 7: Run reth init-state ---
        # Locate chain spec relative to repo root
        repo_root = Path(__file__).resolve().parents[3]
        chain_spec_path = repo_root / DEV_CHAIN_SPEC
        assert chain_spec_path.exists(), f"Chain spec not found: {chain_spec_path}"

        fresh_datadir = output_dir / "reth_init_test"
        run_reth_init_state(jsonl_path, header_path, chain_spec_path, fresh_datadir)
        validate_imported_datadir(fresh_datadir)

        expected_accounts = {
            normalize_hex_address(address): account
            for address, account in state_dump["accounts"].items()
        }
        addresses_to_check = [dev_account.address, funded.address, recipient]
        verify_imported_state_via_rpc(
            fresh_datadir=fresh_datadir,
            chain_spec_path=chain_spec_path,
            expected_block_number=block_num_int,
            expected_state_root=state_root,
            expected_accounts=expected_accounts,
            addresses_to_check=addresses_to_check,
        )
        logger.info("reth init-state import verified successfully (datadir + RPC checks)")

        logger.info("Test complete. Artifacts in %s", output_dir)
        return True
