"""Test native token (ETH) transfers."""

import logging

import flexitest

from common.accounts import get_dev_account
from common.base_test import AlpenClientTest
from common.config.constants import DEV_ADDRESS
from common.evm_utils import create_funded_account, get_balance, wait_for_receipt

logger = logging.getLogger(__name__)

TRANSFER_AMOUNT_WEI = 10**18


@flexitest.register
class TestBalanceTransfer(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        rpc = sequencer.create_rpc()

        dev_account = get_dev_account()
        dev_nonce = int(rpc.eth_getTransactionCount(DEV_ADDRESS, "pending"), 16)
        dev_account.sync_nonce(dev_nonce)

        account = create_funded_account(rpc, dev_account, 10 * 10**18)
        logger.info(f"Created test account: {account.address}")

        recipient = "0x0000000000000000000000000000000000000001"

        source_initial = get_balance(rpc, account.address)
        dest_initial = get_balance(rpc, recipient)

        logger.info(f"Initial balances - Source: {source_initial}, Dest: {dest_initial}")

        gas_price = int(rpc.eth_gasPrice(), 16)

        raw_tx = account.sign_transfer(
            to=recipient,
            value=TRANSFER_AMOUNT_WEI,
            gas_price=gas_price,
            gas=21000,
        )

        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        logger.info(f"Transaction sent: {tx_hash}")

        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", f"Transaction failed: {receipt}"
        logger.info(f"Transaction mined in block {receipt['blockNumber']}")

        source_final = get_balance(rpc, account.address)
        dest_final = get_balance(rpc, recipient)

        logger.info(f"Final balances - Source: {source_final}, Dest: {dest_final}")

        dest_change = dest_final - dest_initial
        assert dest_change == TRANSFER_AMOUNT_WEI, (
            f"Destination balance change {dest_change} != transfer amount {TRANSFER_AMOUNT_WEI}"
        )

        gas_used = int(receipt["gasUsed"], 16)
        gas_cost = gas_used * gas_price
        expected_source_change = -(TRANSFER_AMOUNT_WEI + gas_cost)
        source_change = source_final - source_initial
        assert source_change == expected_source_change, (
            f"Source balance change {source_change} != expected {expected_source_change}"
        )

        logger.info("Balance transfer test passed")
        return True
