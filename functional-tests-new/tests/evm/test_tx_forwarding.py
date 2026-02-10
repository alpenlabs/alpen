"""
Test that verifies transactions sent to a fullnode are forwarded to the sequencer.

This tests the P2P transaction propagation:
1. Send a transaction to the fullnode
2. Verify the sequencer receives and mines it
"""

import logging

import flexitest
from eth_account import Account

from common.accounts import ManagedAccount, get_dev_account
from common.base_test import AlpenClientTest
from common.config.constants import DEV_ADDRESS
from common.wait import wait_until

logger = logging.getLogger(__name__)


def wait_for_receipt(rpc, tx_hash: str, timeout: int = 30) -> dict:
    """Wait for transaction receipt."""
    receipt = None

    def check_receipt():
        nonlocal receipt
        try:
            receipt = rpc.eth_getTransactionReceipt(tx_hash)
            return receipt is not None
        except Exception:
            return False

    wait_until(check_receipt, error_with=f"Transaction {tx_hash} not mined", timeout=timeout)
    return receipt


def create_funded_account(rpc, dev_account: ManagedAccount, amount: int) -> ManagedAccount:
    """Create a new account and fund it from the dev account."""
    new_acct = Account.create()
    new_managed = ManagedAccount(new_acct)

    gas_price = int(rpc.eth_gasPrice(), 16)
    raw_tx = dev_account.sign_transfer(
        to=new_acct.address,
        value=amount,
        gas_price=gas_price,
        gas=25000,
    )
    tx_hash = rpc.eth_sendRawTransaction(raw_tx)
    wait_for_receipt(rpc, tx_hash)

    return new_managed


@flexitest.register
class TestTxForwarding(AlpenClientTest):
    """
    Test that transactions sent to fullnode are forwarded to sequencer.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode = self.get_service("fullnode")

        seq_rpc = sequencer.create_rpc()
        fn_rpc = fullnode.create_rpc()

        # Wait for some blocks to be produced
        sequencer.wait_for_block(2, timeout=30)

        # Create a fresh funded account for this test
        dev_account = get_dev_account()
        # Use "pending" to include unconfirmed transactions (avoids nonce conflicts with parallel tests)
        dev_nonce = int(seq_rpc.eth_getTransactionCount(DEV_ADDRESS, "pending"), 16)
        dev_account.sync_nonce(dev_nonce)

        account = create_funded_account(seq_rpc, dev_account, 10**18)  # 1 ETH
        logger.info(f"Created test account: {account.address}")

        # Wait for fullnode to sync the funded account state
        seq_block = int(seq_rpc.eth_blockNumber(), 16)
        fullnode.wait_for_block(seq_block, timeout=30)
        logger.info(f"Fullnode synced to block {seq_block}")

        # Verify fullnode has the account balance
        fn_balance = int(fn_rpc.eth_getBalance(account.address, "latest"), 16)
        logger.info(f"Fullnode sees balance: {fn_balance} wei")
        assert fn_balance > 0, "Fullnode should see the funded balance"

        # Get gas price from fullnode
        gas_price = int(fn_rpc.eth_gasPrice(), 16)

        # Sign a transaction to a simple address
        recipient = "0x0000000000000000000000000000000000000001"
        raw_tx = account.sign_transfer(
            to=recipient,
            value=1_000_000_000,  # 1 gwei
            gas_price=gas_price,
            gas=25000,
        )

        # Send transaction to FULLNODE (not sequencer)
        logger.info("Sending transaction to fullnode...")
        tx_hash = fn_rpc.eth_sendRawTransaction(raw_tx)
        logger.info(f"Transaction sent to fullnode: {tx_hash}")

        # Wait for receipt from SEQUENCER
        # This proves the tx was forwarded from fullnode -> sequencer
        logger.info("Waiting for receipt from sequencer...")
        receipt = wait_for_receipt(seq_rpc, tx_hash, timeout=30)

        assert receipt is not None, "Transaction not mined"
        assert receipt["status"] == "0x1", f"Transaction failed: {receipt}"

        logger.info(f"Transaction mined in block {receipt['blockNumber']}")
        logger.info("Transaction forwarding test passed")
        return True
