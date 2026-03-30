"""Test migrating legacy chain state via genesis alloc.

Reads the legacy state dump, bakes all accounts into a new genesis JSON `alloc`,
starts the full EE+OL stack with that genesis, and verifies:
  - All imported accounts have correct balances at block 0
  - A transfer between imported accounts succeeds
  - Epoch finalization works

This approach resets block numbers to 0 but requires no code changes to
alpen-client — the normal genesis bootstrap path handles everything.

Prerequisites:
  - Run `cd functional-tests && ./run_test.sh -t el_dump_state` first.
"""

import contextlib
import json
import logging
import os
import tempfile
from pathlib import Path

import flexitest
from eth_utils import to_checksum_address

from common.accounts import ManagedAccount
from common.base_test import BaseTest
from common.config.constants import (
    ALPEN_ACCOUNT_ID,
    DEV_ADDRESS,
    DEV_CHAIN_ID,
    DEV_PRIVATE_KEY,
    ServiceType,
)
from common.evm_utils import get_balance, wait_for_receipt
from common.services.alpen_client import AlpenClientService
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from envconfigs.el_ol import EeOLEnv

logger = logging.getLogger(__name__)

LEGACY_ARTIFACTS_DIR = (
    Path(__file__).resolve().parents[3] / "functional-tests" / "_state_dump_artifacts"
)

# Base dev chain config (same as alpen-dev-chain.json minus the alloc)
DEV_CHAIN_CONFIG = {
    "chainId": 2892,
    "homesteadBlock": 0,
    "eip150Block": 0,
    "eip155Block": 0,
    "eip158Block": 0,
    "byzantiumBlock": 0,
    "constantinopleBlock": 0,
    "petersburgBlock": 0,
    "istanbulBlock": 0,
    "berlinBlock": 0,
    "londonBlock": 0,
    "terminalTotalDifficulty": 0,
    "terminalTotalDifficultyPassed": True,
    "shanghaiTime": 0,
    "cancunTime": 0,
    "pragueTime": 0,
    "mergeNetsplitBlock": 0,
}


def build_genesis_with_alloc(jsonl_path: Path) -> dict:
    """Build a genesis JSON with accounts from the JSONL dump baked into alloc."""
    alloc = {}
    with open(jsonl_path) as f:
        lines = f.readlines()

    # Skip root line, parse accounts
    for line in lines[1:]:
        entry = json.loads(line)
        addr = entry["address"]
        # Strip 0x prefix for alloc keys (reth accepts both but be consistent)
        if addr.startswith("0x"):
            addr = addr[2:]

        account = {}
        balance = entry.get("balance", "0x0")
        if isinstance(balance, str):
            account["balance"] = balance
        else:
            account["balance"] = hex(balance)

        nonce = entry.get("nonce", 0)
        if nonce > 0:
            account["nonce"] = hex(nonce)

        code = entry.get("code", "0x")
        if code and code != "0x":
            account["code"] = code

        storage = entry.get("storage", {})
        if storage:
            account["storage"] = storage

        alloc[addr] = account

    genesis = {
        "nonce": "0x42",
        "timestamp": "0x0",
        "extraData": "0x5343",
        "gasLimit": "0x1c9c380",
        "difficulty": "0x0",
        "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "coinbase": "0x0000000000000000000000000000000000000000",
        "alloc": alloc,
        "number": "0x0",
        "gasUsed": "0x0",
        "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "config": DEV_CHAIN_CONFIG,
    }
    return genesis


def normalize_hex_address(address: str) -> str:
    return address.lower() if address.startswith("0x") else f"0x{address.lower()}"


@flexitest.register
class TestGenesisAllocMigration(BaseTest):
    def __init__(self, ctx: flexitest.InitContext):
        # Load legacy dump and build custom genesis
        assert LEGACY_ARTIFACTS_DIR.is_dir(), (
            f"Legacy artifacts not found at {LEGACY_ARTIFACTS_DIR}. "
            "Run 'cd functional-tests && ./run_test.sh -t el_dump_state' first."
        )
        jsonl_path = LEGACY_ARTIFACTS_DIR / "state_dump.jsonl"
        assert jsonl_path.exists(), f"Missing: {jsonl_path}"

        genesis = build_genesis_with_alloc(jsonl_path)

        # Write genesis JSON to a temp file (must survive until env teardown)
        genesis_fd, genesis_path = tempfile.mkstemp(suffix=".json", prefix="genesis_alloc_")
        self._genesis_path = genesis_path
        with os.fdopen(genesis_fd, "w") as f:
            json.dump(genesis, f, indent=2)
        logger.info("Wrote custom genesis to %s", genesis_path)

        # Parse expected balances from JSONL for verification
        self._expected_accounts = {}
        with open(jsonl_path) as f:
            lines = f.readlines()
        for line in lines[1:]:
            entry = json.loads(line)
            addr = normalize_hex_address(entry["address"])
            self._expected_accounts[addr] = entry

        ctx.set_env(EeOLEnv(pre_generate_blocks=110, custom_chain=genesis_path))

    def main(self, ctx):
        alpen_seq: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        strata_seq: StrataService = self.get_service(ServiceType.Strata)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)

        # --- Phase 1: Wait for services ---
        logger.info("Waiting for services...")
        strata_seq.wait_for_rpc_ready(timeout=20)
        strata_seq.wait_for_account_genesis_epoch_commitment(
            ALPEN_ACCOUNT_ID,
            timeout=20,
        )
        alpen_seq.wait_for_block(1, timeout=60)

        rpc = alpen_seq.create_rpc()
        block_num = int(rpc.eth_blockNumber(), 16)
        logger.info("Chain started, at block %d", block_num)

        # --- Phase 2: Verify imported balances ---
        logger.info("Verifying %d imported accounts...", len(self._expected_accounts))
        for addr, acct in self._expected_accounts.items():
            expected_bal = int(acct.get("balance", "0x0"), 16)
            actual_bal = get_balance(rpc, addr)
            logger.info("  %s: expected %d, got %d", addr, expected_bal, actual_bal)
            assert actual_bal == expected_bal, (
                f"Balance mismatch for {addr}: expected {expected_bal}, got {actual_bal}"
            )
        logger.info("All imported balances verified")

        # --- Phase 3: Send a transfer ---
        # Find a non-dev funded account as recipient
        recipient_address = None
        for addr, acct in self._expected_accounts.items():
            if addr.lower() != DEV_ADDRESS.lower():
                if int(acct.get("balance", "0x0"), 16) > 0:
                    recipient_address = addr
                    break
        assert recipient_address is not None, "No funded non-dev account"

        dev_account = ManagedAccount.from_key(DEV_PRIVATE_KEY, chain_id=DEV_CHAIN_ID)
        nonce = int(rpc.eth_getTransactionCount(dev_account.address, "pending"), 16)
        dev_account.sync_nonce(nonce)

        transfer_amount = 10**18  # 1 ETH
        gas_price = int(rpc.eth_gasPrice(), 16)

        recipient_before = get_balance(rpc, recipient_address)

        recipient_checksum = to_checksum_address(recipient_address)
        logger.info("Sending 1 ETH from %s to %s...", DEV_ADDRESS, recipient_checksum)
        raw_tx = dev_account.sign_transfer(
            to=recipient_checksum,
            value=transfer_amount,
            gas_price=gas_price,
            gas=21000,
        )
        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", f"Transfer failed: {receipt}"
        logger.info("Transfer mined in block %s", receipt["blockNumber"])

        recipient_after = get_balance(rpc, recipient_address)
        assert recipient_after == recipient_before + transfer_amount, (
            f"Recipient balance wrong: expected {recipient_before + transfer_amount}, "
            f"got {recipient_after}"
        )
        logger.info("Transfer verified: recipient %d -> %d", recipient_before, recipient_after)

        # --- Phase 4: Wait for the transfer block to be finalized ---
        tx_block_num = int(receipt["blockNumber"], 16)
        tx_block = rpc.eth_getBlockByNumber(hex(tx_block_num), False)
        tx_block_hash = tx_block["hash"]

        # Verify the transfer block starts as pending
        initial_status = alpen_seq.get_block_status(tx_block_hash)
        logger.info("Transfer block %d status: %s", tx_block_num, initial_status)

        # Mine L1 blocks until the transfer block is finalized
        logger.info("Mining L1 blocks until transfer block %d is finalized...", tx_block_num)
        final_status = bitcoin.mine_until(
            check=lambda: alpen_seq.get_block_status(tx_block_hash),
            predicate=lambda s: s == "finalized",
            error_with=f"Transfer block {tx_block_num} did not reach finalized status",
            timeout=120,
        )
        logger.info("Transfer block %d status: %s", tx_block_num, final_status)

        # Verify balances are still correct after finalization
        recipient_final = get_balance(rpc, recipient_address)
        assert recipient_final == recipient_before + transfer_amount, (
            f"Recipient balance changed after finalization: "
            f"expected {recipient_before + transfer_amount}, got {recipient_final}"
        )
        logger.info("Balances stable after finalization")

        logger.info(
            "Test complete: genesis alloc migration works — "
            "imported balances correct, transfer finalized on L1"
        )

        # Cleanup temp file
        with contextlib.suppress(OSError):
            os.unlink(self._genesis_path)

        return True
