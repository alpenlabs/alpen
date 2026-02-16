"""Test that EIP-4844 blob transactions are rejected."""

import logging
import random

import flexitest
from eth_abi import abi
from eth_account import Account

from common.base_test import AlpenClientTest
from common.config.constants import DEV_CHAIN_ID
from common.rpc import RpcError

logger = logging.getLogger(__name__)

BLOB_TX_TYPE = 3
BLOB_SIZE = 4096
BLOB_CHUNK_SIZE = 32

EXPECTED_ERROR_CODE = -32003
EXPECTED_ERROR_MESSAGE = "transaction type not supported"


def create_blob_data() -> bytes:
    text = "<( o.O )>"
    encoded_text = abi.encode(["string"], [text])
    padding_size = BLOB_CHUNK_SIZE * (BLOB_SIZE - len(encoded_text) // BLOB_CHUNK_SIZE)
    return (b"\x00" * padding_size) + encoded_text


@flexitest.register
class TestBlobTransactionRejected(AlpenClientTest):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("alpen_client")

    def main(self, ctx):
        sequencer = self.get_service("sequencer")
        rpc = sequencer.create_rpc()

        private_key = hex(random.getrandbits(256))
        account = Account.from_key(private_key)

        nonce = int(rpc.eth_getTransactionCount(account.address, "latest"), 16)

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

        blob_data = create_blob_data()
        signed_tx = account.sign_transaction(tx, blobs=[blob_data])
        raw_tx = "0x" + signed_tx.raw_transaction.hex()

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
