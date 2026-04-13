"""Test transaction forwarding from fullnode to sequencer."""

import logging

import flexitest

from common.accounts import get_dev_account
from common.base_test import AlpenClientTest
from common.config.constants import ServiceType
from common.evm_utils import create_funded_account, send_raw_transaction, wait_for_receipt

logger = logging.getLogger(__name__)


@flexitest.register
class TestTxForwarding(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        ee_sequencer = self.get_service(ServiceType.AlpenSequencer)
        ee_fullnode = self.get_service(ServiceType.AlpenFullNode)

        seq_rpc = ee_sequencer.create_rpc()
        fn_rpc = ee_fullnode.create_rpc()

        ee_sequencer.wait_for_block(2)

        dev_account = get_dev_account(seq_rpc)
        account = create_funded_account(seq_rpc, dev_account, 10**18)
        logger.info(f"Created test account: {account.address}")

        seq_block = int(seq_rpc.eth_blockNumber(), 16)
        ee_fullnode.wait_for_block(seq_block)
        logger.info(f"Fullnode synced to block {seq_block}")

        fn_balance = int(fn_rpc.eth_getBalance(account.address, "latest"), 16)
        logger.info(f"Fullnode sees balance: {fn_balance} wei")
        assert fn_balance > 0, "Fullnode should see the funded balance"

        gas_price = int(fn_rpc.eth_gasPrice(), 16)

        recipient = "0x000000000000000000000000000000000000dEaD"
        raw_tx = account.sign_transfer(
            to=recipient,
            value=1_000_000_000,
            gas_price=gas_price,
            gas=21000,
        )

        logger.info("Sending transaction to fullnode...")
        tx_hash = send_raw_transaction(fn_rpc, raw_tx)
        logger.info(f"Transaction sent to fullnode: {tx_hash}")

        logger.info("Waiting for receipt from sequencer...")
        receipt = wait_for_receipt(seq_rpc, tx_hash)

        assert receipt is not None, "Transaction not mined"
        assert receipt["status"] == "0x1", f"Transaction failed: {receipt}"

        logger.info(f"Transaction mined in block {receipt['blockNumber']}")
        logger.info("Transaction forwarding test passed")
        return True
