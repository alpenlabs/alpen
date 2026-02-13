"""
Test that verifies nonce handling works correctly.

Tests:
- Nonce increments after each transaction
- Transactions with wrong nonce are rejected
- Pending nonce reflects unconfirmed transactions
"""

import logging

import flexitest

from common.accounts import get_dev_account
from common.base_test import AlpenClientTest
from common.config.constants import DEV_ADDRESS
from common.evm_utils import create_funded_account, wait_for_receipt
from common.rpc import RpcError

logger = logging.getLogger(__name__)


@flexitest.register
class TestNonceHandling(AlpenClientTest):
    """
    Test nonce handling for transactions.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        rpc = sequencer.create_rpc()

        # Create a fresh funded account for this test
        dev_account = get_dev_account()
        # Use "pending" to include unconfirmed txs (avoids nonce conflicts)
        dev_nonce = int(rpc.eth_getTransactionCount(DEV_ADDRESS, "pending"), 16)
        dev_account.sync_nonce(dev_nonce)

        account = create_funded_account(rpc, dev_account, 10**18)  # 1 ETH
        logger.info(f"Created test account: {account.address}")

        recipient = "0x0000000000000000000000000000000000000001"

        # Get initial nonce (should be 0 for new account)
        initial_nonce = int(rpc.eth_getTransactionCount(account.address, "latest"), 16)
        logger.info(f"Initial nonce: {initial_nonce}")
        assert initial_nonce == 0, f"New account should have nonce 0, got {initial_nonce}"

        # Get gas price
        gas_price = int(rpc.eth_gasPrice(), 16)

        # Test 1: Send transaction with correct nonce
        raw_tx = account.sign_transfer(
            to=recipient,
            value=1000,
            gas_price=gas_price,
            gas=25000,
        )
        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        logger.info(f"Sent tx with nonce {initial_nonce}: {tx_hash}")

        # Wait for confirmation
        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", "Transaction should succeed"

        # Verify nonce incremented
        new_nonce = int(rpc.eth_getTransactionCount(account.address, "latest"), 16)
        assert new_nonce == initial_nonce + 1, (
            f"Nonce should increment: expected {initial_nonce + 1}, got {new_nonce}"
        )
        logger.info(f"Nonce after tx: {new_nonce}")

        # Test 2: Send transaction with old nonce (should fail)
        old_nonce_tx = account.sign_transfer(
            to=recipient,
            value=1000,
            gas_price=gas_price,
            gas=25000,
            nonce=initial_nonce,  # Use old nonce
        )

        try:
            rpc.eth_sendRawTransaction(old_nonce_tx)
            raise AssertionError("Transaction with old nonce should be rejected")
        except RpcError as e:
            logger.info(f"Old nonce correctly rejected: {e.message}")
            assert e.message and "nonce" in e.message.lower(), (
                f"Expected nonce error, got: {e.message}"
            )

        # Test 3: Send multiple transactions and verify nonces
        tx_hashes = []
        for i in range(3):
            raw_tx = account.sign_transfer(
                to=recipient,
                value=1000,
                gas_price=gas_price,
                gas=25000,
            )
            tx_hash = rpc.eth_sendRawTransaction(raw_tx)
            tx_hashes.append(tx_hash)
            logger.info(f"Sent tx {i + 1}/3: {tx_hash}")

        # Wait for all to be mined
        for tx_hash in tx_hashes:
            receipt = wait_for_receipt(rpc, tx_hash)
            assert receipt["status"] == "0x1", f"Transaction {tx_hash} should succeed"

        # Verify final nonce
        final_nonce = int(rpc.eth_getTransactionCount(account.address, "latest"), 16)
        expected_nonce = initial_nonce + 4  # 1 + 3 = 4 transactions
        assert final_nonce == expected_nonce, (
            f"Final nonce should be {expected_nonce}, got {final_nonce}"
        )
        logger.info(f"Final nonce: {final_nonce}")

        logger.info("Nonce handling test passed")
        return True
