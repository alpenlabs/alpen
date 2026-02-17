"""Test transaction forwarding from fullnode to sequencer."""

import logging

import flexitest

from common.accounts import get_dev_account
from common.base_test import AlpenClientTest
from common.config.constants import DEV_ADDRESS
from common.evm_utils import create_funded_account, wait_for_receipt

logger = logging.getLogger(__name__)


@flexitest.register
class TestTxForwarding(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        fullnode = self.get_service("fullnode")

        seq_rpc = sequencer.create_rpc()
        fn_rpc = fullnode.create_rpc()

        sequencer.wait_for_block(2, timeout=10)

        dev_account = get_dev_account()
        dev_nonce = int(seq_rpc.eth_getTransactionCount(DEV_ADDRESS, "pending"), 16)
        dev_account.sync_nonce(dev_nonce)

        account = create_funded_account(seq_rpc, dev_account, 10**18)
        logger.info(f"Created test account: {account.address}")

        seq_block = int(seq_rpc.eth_blockNumber(), 16)
        fullnode.wait_for_block(seq_block, timeout=10)
        logger.info(f"Fullnode synced to block {seq_block}")

        fn_balance = int(fn_rpc.eth_getBalance(account.address, "latest"), 16)
        logger.info(f"Fullnode sees balance: {fn_balance} wei")
        assert fn_balance > 0, "Fullnode should see the funded balance"

        gas_price = int(fn_rpc.eth_gasPrice(), 16)

        recipient = "0x0000000000000000000000000000000000000001"
        raw_tx = account.sign_transfer(
            to=recipient,
            value=1_000_000_000,
            gas_price=gas_price,
            gas=21000,
        )

        logger.info("Sending transaction to fullnode...")
        tx_hash = fn_rpc.eth_sendRawTransaction(raw_tx)
        logger.info(f"Transaction sent to fullnode: {tx_hash}")

        logger.info("Waiting for receipt from sequencer...")
        receipt = wait_for_receipt(seq_rpc, tx_hash)

        assert receipt is not None, "Transaction not mined"
        assert receipt["status"] == "0x1", f"Transaction failed: {receipt}"

        logger.info(f"Transaction mined in block {receipt['blockNumber']}")
        logger.info("Transaction forwarding test passed")
        return True
