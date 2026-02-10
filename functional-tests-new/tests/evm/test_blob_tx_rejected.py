"""
Test that verifies EIP-4844 blob transactions are rejected.

Alpen EVM does not support blob transactions. This test ensures they
are properly rejected with the expected error.
"""

import logging
import random

import flexitest
from eth_abi import abi
from eth_account import Account

from common.base_test import AlpenClientTest
from common.config.constants import DEV_CHAIN_ID
from common.rpc import RpcError

logger = logging.getLogger(__name__)

# EIP-4844 blob transaction configuration
BLOB_TX_TYPE = 3
BLOB_SIZE = 4096
BLOB_CHUNK_SIZE = 32

# Expected error when blob tx is rejected
EXPECTED_ERROR_CODE = -32003
EXPECTED_ERROR_MESSAGE = "transaction type not supported"


def create_blob_data() -> bytes:
    """Create padded blob data with encoded test string."""
    text = "<( o.O )>"
    encoded_text = abi.encode(["string"], [text])

    # Calculate padding to reach standard blob size
    padding_size = BLOB_CHUNK_SIZE * (BLOB_SIZE - len(encoded_text) // BLOB_CHUNK_SIZE)
    return (b"\x00" * padding_size) + encoded_text


@flexitest.register
class TestBlobTransactionRejected(AlpenClientTest):
    """
    Verify that Alpen EVM correctly rejects EIP-4844 blob transactions.

    Blob transactions (type 3) are not supported and should fail with
    a "transaction type not supported" error.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        rpc = sequencer.create_rpc()

        # Generate a random account for the test
        private_key = hex(random.getrandbits(256))
        account = Account.from_key(private_key)

        # Get chain ID and nonce
        nonce = int(rpc.eth_getTransactionCount(account.address, "latest"), 16)

        # Build EIP-4844 blob transaction
        tx = {
            "type": BLOB_TX_TYPE,
            "chainId": DEV_CHAIN_ID,
            "from": account.address,
            "to": "0x0000000000000000000000000000000000000000",
            "value": 0,
            "nonce": nonce,
            "maxFeePerGas": 10**12,
            "maxPriorityFeePerGas": 10**12,
            "maxFeePerBlobGas": 10**12,
            "gas": 100000,
        }

        # Sign with blob data
        blob_data = create_blob_data()
        signed_tx = account.sign_transaction(tx, blobs=[blob_data])
        raw_tx = "0x" + signed_tx.raw_transaction.hex()

        # Attempt to send - should fail
        logger.info("Sending blob transaction (expecting rejection)...")
        try:
            rpc.eth_sendRawTransaction(raw_tx)
            raise AssertionError("Blob transaction should have been rejected")
        except RpcError as e:
            logger.info(f"Transaction rejected as expected: {e.code} - {e.message}")
            assert e.code == EXPECTED_ERROR_CODE, (
                f"Expected error code {EXPECTED_ERROR_CODE}, got {e.code}"
            )
            assert EXPECTED_ERROR_MESSAGE in e.message, (
                f"Expected '{EXPECTED_ERROR_MESSAGE}' in error message, got: {e.message}"
            )

        logger.info("Blob transaction rejection test passed")
        return True
