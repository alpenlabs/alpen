"""
Test transaction propagation from fullnode to sequencer.

Verifies that transactions sent to a fullnode are forwarded to the sequencer
via --sequencer-http, included in a block, and propagated back to all fullnodes.
"""

import logging

import flexitest

from common.accounts import get_dev_account, get_recipient_account
from common.base_test import AlpenClientTest
from common.wait import wait_until

logger = logging.getLogger(__name__)

TX_VALUE_WEI = 10**15  # 0.001 ETH


@flexitest.register
class TestTransactionMempoolPropagation(AlpenClientTest):
    """Test that transactions sent to fullnode get mined and propagated."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client_multi")

    def main(self, ctx):  # noqa: ARG002
        sequencer = self.get_service("sequencer")
        fullnodes = [self.get_service(f"fullnode_{i}") for i in range(3)]

        # Wait for P2P mesh to form
        sequencer.wait_for_peers(3, timeout=60)
        for fn in fullnodes:
            fn.wait_for_peers(1, timeout=30)

        # Wait for chain to be active
        sequencer.wait_for_block(5, timeout=60)

        seq_rpc = sequencer.create_rpc()
        fn_rpc = fullnodes[0].create_rpc()

        dev_account = get_dev_account()
        recipient_account = get_recipient_account()

        # Verify dev account has funds
        balance = int(seq_rpc.eth_getBalance(dev_account.address, "latest"), 16)
        assert balance > 0, "Dev account has no balance"

        # Build and send transaction to fullnode (not sequencer)
        nonce = int(seq_rpc.eth_getTransactionCount(dev_account.address, "pending"), 16)
        dev_account.sync_nonce(nonce)
        gas_price = int(int(seq_rpc.eth_gasPrice(), 16) * 1.5)

        raw_tx = dev_account.sign_transfer(
            to=recipient_account.address,
            value=TX_VALUE_WEI,
            gas_price=gas_price,
        )

        tx_hash = fn_rpc.eth_sendRawTransaction(raw_tx)
        logger.info(f"Sent tx {tx_hash} to fullnode_0")

        # Wait for transaction to be mined
        def tx_mined():
            receipt = seq_rpc.eth_getTransactionReceipt(tx_hash)
            return receipt is not None

        wait_until(tx_mined, error_with=f"Transaction {tx_hash} not mined", timeout=120)

        receipt = seq_rpc.eth_getTransactionReceipt(tx_hash)
        block_num = int(receipt["blockNumber"], 16)
        assert int(receipt["status"], 16) == 1, "Transaction failed"
        logger.info(f"Transaction mined in block {block_num}")

        # Verify block propagated to all fullnodes with correct tx
        seq_block = sequencer.get_block_by_number(block_num)
        for i, fn in enumerate(fullnodes):
            fn.wait_for_block(block_num, timeout=60)
            fn_block = fn.get_block_by_number(block_num)
            assert fn_block["hash"] == seq_block["hash"], f"Fullnode {i} hash mismatch"
            assert tx_hash.lower() in [t.lower() for t in fn_block["transactions"]], (
                f"Tx missing from fullnode {i}"
            )

        # Verify recipient received funds
        recipient_balance = int(seq_rpc.eth_getBalance(recipient_account.address, "latest"), 16)
        assert recipient_balance >= TX_VALUE_WEI, "Recipient didn't receive funds"

        logger.info(f"Transaction propagation verified: block {block_num}")
        return True
