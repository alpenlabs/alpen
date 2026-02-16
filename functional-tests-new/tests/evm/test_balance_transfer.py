"""Test native token (ETH) transfers."""

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

        original_block = sequencer.get_block_number()
        source_original = get_balance(rpc, account.address)
        dest_original = get_balance(rpc, recipient)
        basefee_original = get_balance(rpc, BASEFEE_ADDRESS)
        beneficiary_original = get_balance(rpc, BENEFICIARY_ADDRESS)

        logger.info(f"Original balances - Source: {source_original}, Dest: {dest_original}")

        gas_price = int(rpc.eth_gasPrice(), 16)
        logger.info(f"Gas price: {gas_price} wei")

        raw_tx = account.sign_transfer(
            to=recipient,
            value=TRANSFER_AMOUNT_WEI,
            gas_price=gas_price,
            gas=25000,
        )

        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        logger.info(f"Transaction sent: {tx_hash}")

        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", f"Transaction failed: {receipt}"
        logger.info(f"Transaction mined in block {receipt['blockNumber']}")

        final_block = sequencer.get_block_number()
        source_final = get_balance(rpc, account.address)
        dest_final = get_balance(rpc, recipient)
        basefee_final = get_balance(rpc, BASEFEE_ADDRESS)
        beneficiary_final = get_balance(rpc, BENEFICIARY_ADDRESS)

        logger.info(f"Final balances - Source: {source_final}, Dest: {dest_final}")

        assert final_block > original_block, "Block number should have advanced"

        dest_change = dest_final - dest_original
        assert dest_change == TRANSFER_AMOUNT_WEI, (
            f"Destination balance change {dest_change} != transfer amount {TRANSFER_AMOUNT_WEI}"
        )

        basefee_change = basefee_final - basefee_original
        beneficiary_change = beneficiary_final - beneficiary_original
        logger.info(f"Basefee change: {basefee_change}, Beneficiary change: {beneficiary_change}")

        assert basefee_change >= 0, "Basefee balance should not decrease"
        assert beneficiary_change >= 0, "Beneficiary balance should not decrease"

        source_change = source_final - source_original
        total_change = source_change + dest_change + basefee_change + beneficiary_change

        assert abs(total_change) < GWEI_TO_WEI, (
            f"Balance not conserved: total change = {total_change}"
        )

        logger.info("Balance transfer test passed")
        return True
