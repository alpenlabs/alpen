"""Test nonce handling for transactions."""

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
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_ee")

    def main(self, ctx):
        ee_sequencer = self.get_service("ee_sequencer")
        rpc = ee_sequencer.create_rpc()

        dev_account = get_dev_account()
        dev_nonce = int(rpc.eth_getTransactionCount(DEV_ADDRESS, "pending"), 16)
        dev_account.sync_nonce(dev_nonce)

        account = create_funded_account(rpc, dev_account, 10**18)
        logger.info(f"Created test account: {account.address}")

        recipient = "0x000000000000000000000000000000000000dEaD"

        initial_nonce = int(rpc.eth_getTransactionCount(account.address, "latest"), 16)
        logger.info(f"Initial nonce: {initial_nonce}")
        assert initial_nonce == 0, f"New account should have nonce 0, got {initial_nonce}"

        gas_price = int(rpc.eth_gasPrice(), 16)

        raw_tx = account.sign_transfer(
            to=recipient,
            value=1000,
            gas_price=gas_price,
            gas=21000,
        )
        tx_hash = rpc.eth_sendRawTransaction(raw_tx)
        logger.info(f"Sent tx with nonce 0: {tx_hash}")

        receipt = wait_for_receipt(rpc, tx_hash)
        assert receipt["status"] == "0x1", "Transaction should succeed"

        new_nonce = int(rpc.eth_getTransactionCount(account.address, "latest"), 16)
        assert new_nonce == 1, f"Nonce should be 1, got {new_nonce}"
        logger.info(f"Nonce after tx: {new_nonce}")

        try:
            rpc.eth_sendRawTransaction(raw_tx)
            raise AssertionError("Replaying tx with old nonce should be rejected")
        except RpcError as e:
            logger.info(f"Replay correctly rejected: {e.message}")
            assert e.message and "nonce" in e.message.lower(), (
                f"Expected nonce error, got: {e.message}"
            )

        tx_hashes = []
        for i in range(3):
            raw_tx = account.sign_transfer(
                to=recipient,
                value=1000,
                gas_price=gas_price,
                gas=21000,
            )
            tx_hash = rpc.eth_sendRawTransaction(raw_tx)
            tx_hashes.append(tx_hash)
            logger.info(f"Sent tx {i + 1}/3: {tx_hash}")

        for tx_hash in tx_hashes:
            receipt = wait_for_receipt(rpc, tx_hash)
            assert receipt["status"] == "0x1", f"Transaction {tx_hash} should succeed"

        final_nonce = int(rpc.eth_getTransactionCount(account.address, "latest"), 16)
        assert final_nonce == 4, f"Final nonce should be 4, got {final_nonce}"
        logger.info(f"Final nonce: {final_nonce}")

        logger.info("Nonce handling test passed")
        return True
