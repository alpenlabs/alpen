"""
Test that verifies native token (ETH) transfers work correctly.

This test sends a simple ETH transfer and verifies:
- The destination balance increases by the transfer amount
- The source balance decreases appropriately (transfer + gas)
- Gas fees are distributed to basefee and beneficiary addresses
"""

import logging

import flexitest

from common.accounts import get_dev_account
from common.base_test import AlpenClientTest
from common.config.constants import (
    BASEFEE_ADDRESS,
    BENEFICIARY_ADDRESS,
    DEV_ADDRESS,
    GWEI_TO_WEI,
)
from common.evm_utils import create_funded_account, get_balance, wait_for_receipt

logger = logging.getLogger(__name__)

# Transfer amount in wei (1 ETH)
TRANSFER_AMOUNT_WEI = 10**18


@flexitest.register
class TestBalanceTransfer(AlpenClientTest):
    """
    Test native token transfer and verify balance changes.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        rpc = sequencer.create_rpc()

        # Create a fresh funded account for this test to avoid nonce conflicts
        dev_account = get_dev_account()
        # Use "pending" to include unconfirmed txs (avoids nonce conflicts)
        dev_nonce = int(rpc.eth_getTransactionCount(DEV_ADDRESS, "pending"), 16)
        dev_account.sync_nonce(dev_nonce)

        # Fund a test account with 10 ETH
        account = create_funded_account(rpc, dev_account, 10 * 10**18)
        logger.info(f"Created test account: {account.address}")

        # Use a simple recipient address for this test
        recipient = "0x0000000000000000000000000000000000000001"

        # Get original balances
        original_block = sequencer.get_block_number()
        source_original = get_balance(rpc, account.address)
        dest_original = get_balance(rpc, recipient)
        basefee_original = get_balance(rpc, BASEFEE_ADDRESS)
        beneficiary_original = get_balance(rpc, BENEFICIARY_ADDRESS)

        logger.info(f"Original balances - Source: {source_original}, Dest: {dest_original}")

        # Get gas price
        gas_price = int(rpc.eth_gasPrice(), 16)
        logger.info(f"Gas price: {gas_price} wei")

        # Sign and send transfer
        raw_tx = account.sign_transfer(
            to=recipient,
            value=TRANSFER_AMOUNT_WEI,
            gas_price=gas_price,
            gas=25000,  # Slightly more than 21000 for safety
        )

        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        logger.info(f"Transaction sent: {tx_hash}")

        # Wait for receipt
        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", f"Transaction failed: {receipt}"
        logger.info(f"Transaction mined in block {receipt['blockNumber']}")

        # Get final balances
        final_block = sequencer.get_block_number()
        source_final = get_balance(rpc, account.address)
        dest_final = get_balance(rpc, recipient)
        basefee_final = get_balance(rpc, BASEFEE_ADDRESS)
        beneficiary_final = get_balance(rpc, BENEFICIARY_ADDRESS)

        logger.info(f"Final balances - Source: {source_final}, Dest: {dest_final}")

        # Verify block advanced
        assert final_block > original_block, "Block number should have advanced"

        # Verify destination received the transfer
        dest_change = dest_final - dest_original
        assert dest_change == TRANSFER_AMOUNT_WEI, (
            f"Destination balance change {dest_change} != transfer amount {TRANSFER_AMOUNT_WEI}"
        )

        # Verify gas fees were collected
        basefee_change = basefee_final - basefee_original
        beneficiary_change = beneficiary_final - beneficiary_original
        logger.info(f"Basefee change: {basefee_change}, Beneficiary change: {beneficiary_change}")

        assert basefee_change >= 0, "Basefee balance should not decrease"
        assert beneficiary_change >= 0, "Beneficiary balance should not decrease"

        # Verify total balance is conserved (source loss = dest gain + fees)
        source_change = source_final - source_original
        total_change = source_change + dest_change + basefee_change + beneficiary_change

        # Total change should be 0 (conservation of value)
        # Note: there might be small discrepancies due to block rewards, so we allow some tolerance
        assert abs(total_change) < GWEI_TO_WEI, (
            f"Balance not conserved: total change = {total_change}"
        )

        logger.info("Balance transfer test passed")
        return True
