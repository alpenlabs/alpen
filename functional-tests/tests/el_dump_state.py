"""Dump EE state from legacy chain for migration to new chain via `reth init-state`.

Creates EVM state (balance transfer), waits for epoch finalization,
then dumps the complete EE state and RLP header to a shared artifacts directory.

Follows the same env setup pattern as `el_sync_from_chainstate.py`:
  - ProverClientSettings.new_with_proving() for checkpoint proofs
  - get_fast_batch_settings() for fast epoch/proof timeouts

The artifacts are written to a well-known location that the new functional tests
(`functional-tests-new/`) can pick up for import validation.

Artifacts:
  - state_dump.jsonl   — JSONL state dump (reth init-state format)
  - header.rlp         — RLP-encoded block header at dump point
  - dump_info.json     — metadata (block number, state root, balances)
"""

import json
import os

import flexitest
from web3 import Web3

from envs import net_settings, testenv
from factory import seqrpc
from mixins import BaseMixin
from utils import ProverClientSettings

# Shared artifacts directory — readable by functional-tests-new
ARTIFACTS_DIR = os.path.join(os.path.dirname(os.path.dirname(__file__)), "_state_dump_artifacts")


def write_state_dump_jsonl(state_dump: dict, path: str) -> int:
    """Convert strataee_dumpState response to JSONL format for reth init-state."""
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


def validate_jsonl_format(path: str) -> int:
    """Validate JSONL file is well-formed. Returns account count."""
    with open(path) as f:
        lines = f.readlines()
    assert len(lines) >= 1, "JSONL file is empty"
    root_line = json.loads(lines[0])
    assert "root" in root_line, f"First line missing 'root': {root_line}"
    count = 0
    for i, line in enumerate(lines[1:], start=2):
        entry = json.loads(line)
        assert "address" in entry, f"Line {i} missing 'address'"
        assert "balance" in entry, f"Line {i} missing 'balance'"
        assert "nonce" in entry, f"Line {i} missing 'nonce'"
        count += 1
    return count


@flexitest.register
class ElDumpStateTest(BaseMixin):
    def __init__(self, ctx: flexitest.InitContext):
        # Match el_sync_from_chainstate.py setup:
        # - ProverClientSettings.new_with_proving() for checkpoint proofs
        # - get_fast_batch_settings() for fast epoch/proof timeouts
        ctx.set_env(
            testenv.BasicEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        w3: Web3 = self.w3
        strata_waiter = self.create_strata_waiter(self.seqrpc, timeout=60)

        # Wait for genesis
        strata_waiter.wait_until_genesis()
        self.info("Genesis reached")

        # --- Phase 1: Create EVM state ---
        self.info("Creating EVM state...")
        source = w3.address
        dest = w3.to_checksum_address("0x000000000000000000000000000000000000dEaD")

        # Send a few txs (workaround for reth restart issue, same as el_sync_from_chainstate)
        for _ in range(3):
            tx_hash = w3.eth.send_transaction({
                "from": source,
                "to": dest,
                "value": Web3.to_wei(1, "ether"),
            })
            w3.eth.wait_for_transaction_receipt(tx_hash, timeout=30)

        self.info("Transactions mined")

        # --- Phase 2: Wait for epoch 0 finalization ---
        self.info("Waiting for epoch 0 to finalize...")
        strata_waiter.wait_until_epoch_finalized(0, timeout=30)
        self.info("Epoch 0 finalized")

        # --- Phase 3: Record balances ---
        source_balance = w3.eth.get_balance(source)
        dest_balance = w3.eth.get_balance(dest)
        self.info(f"Balances — source: {source_balance}, dest: {dest_balance}")

        # --- Phase 4: Dump state via strataee_dumpState ---
        eth_rpc_http_port = self.reth.get_prop("eth_rpc_http_port")
        http_rpc = seqrpc.JsonrpcClient(f"http://localhost:{eth_rpc_http_port}")

        self.info("Dumping state at latest block...")
        state_dump = http_rpc.strataee_dumpState(None)
        assert state_dump is not None, "strataee_dumpState returned null"

        account_count = len(state_dump["accounts"])
        state_root = state_dump["root"]
        block_num = state_dump["block_number"]
        self.info(f"State dump: block {block_num}, {account_count} accounts, root: {state_root}")

        # Verify known accounts are in dump
        dump_addrs = {a.lower() for a in state_dump["accounts"]}
        assert source.lower() in dump_addrs, f"Source {source} not in dump"

        # Get RLP header
        block_hex = hex(block_num)
        raw_header = http_rpc.debug_getRawHeader(block_hex)
        header_hex = raw_header[2:] if raw_header.startswith("0x") else raw_header
        self.info(f"Raw header: {len(header_hex) // 2} bytes")

        # Verify state root consistency
        block_info = http_rpc.eth_getBlockByNumber(block_hex, False)
        block_hash = block_info["hash"]
        assert block_info["stateRoot"] == state_root, (
            f"State root mismatch: dump={state_root}, header={block_info['stateRoot']}"
        )

        # --- Phase 5: Write artifacts ---
        os.makedirs(ARTIFACTS_DIR, exist_ok=True)

        jsonl_path = os.path.join(ARTIFACTS_DIR, "state_dump.jsonl")
        written = write_state_dump_jsonl(state_dump, jsonl_path)
        self.info(f"Wrote {written} accounts to {jsonl_path}")

        header_path = os.path.join(ARTIFACTS_DIR, "header.rlp")
        with open(header_path, "w") as f:
            f.write(header_hex)
        self.info(f"Wrote header to {header_path}")

        dump_info = {
            "block_number": block_num,
            "block_hash": block_hash,
            "state_root": state_root,
            "account_count": account_count,
            "source_chain": "legacy",
            "chain_spec": "dev",
            "balances": {
                source: str(source_balance),
                dest: str(dest_balance),
            },
            "artifacts": {
                "state_dump": jsonl_path,
                "header": header_path,
            },
        }
        info_path = os.path.join(ARTIFACTS_DIR, "dump_info.json")
        with open(info_path, "w") as f:
            json.dump(dump_info, f, indent=2)
        self.info(f"Wrote dump info to {info_path}")

        # --- Phase 6: Validate format ---
        validated = validate_jsonl_format(jsonl_path)
        assert validated == written, f"Validation mismatch: wrote {written}, validated {validated}"
        self.info(f"JSONL validated: {validated} accounts")

        self.info(f"Legacy state dump complete. Artifacts in {ARTIFACTS_DIR}")
        return True
