import random

import flexitest
from eth_abi import abi
from web3 import Web3
from web3.exceptions import Web3RPCError

from mixins import BaseMixin

# EIP-4844 blob transaction configuration
TRANSACTION_CONFIG = {
    "TYPE": 3,  # EIP-4844 blob transaction type
    "TO_ADDRESS": "0x0000000000000000000000000000000000000000",
    "VALUE": 0,
    "MAX_FEE_PER_GAS": 10**12,
    "MAX_PRIORITY_FEE_PER_GAS": 10**12,
    "MAX_FEE_PER_BLOB_GAS": hex(10**12),
    "GAS_LIMIT": 100000,
}

EXPECTED_ERROR = "{'code': -32003, 'message': 'transaction type not supported'}"
BLOB_SIZE = 4096
BLOB_CHUNK_SIZE = 32


@flexitest.register
class ElBalanceTransferTest(BaseMixin):
    """
    Test that Alpen EVM correctly rejects EIP-4844 blob transactions.
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, _ctx: flexitest.RunContext):
        """Test blob transaction rejection on Alpen EVM"""
        web3 = self.w3
        new_account = self._generate_random_account(web3)
        transaction = self._build_blob_transaction(web3, new_account.address)
        signed_tx = self._sign_with_blob_data(transaction, new_account)
        self._assert_transaction_rejected(web3, signed_tx)

    def _generate_random_account(self, web3: Web3):
        """Generate account with cryptographically secure random private key"""
        private_key = hex(random.getrandbits(256))
        return web3.eth.account.from_key(private_key)

    def _build_blob_transaction(self, web3: Web3, from_address: str) -> dict:
        """Build EIP-4844 blob transaction structure for rejection test"""
        return {
            "type": TRANSACTION_CONFIG["TYPE"],
            "chainId": web3.eth.chain_id,
            "from": from_address,
            "to": TRANSACTION_CONFIG["TO_ADDRESS"],
            "value": TRANSACTION_CONFIG["VALUE"],
            "nonce": web3.eth.get_transaction_count(from_address),
            "maxFeePerGas": TRANSACTION_CONFIG["MAX_FEE_PER_GAS"],
            "maxPriorityFeePerGas": TRANSACTION_CONFIG["MAX_PRIORITY_FEE_PER_GAS"],
            "maxFeePerBlobGas": TRANSACTION_CONFIG["MAX_FEE_PER_BLOB_GAS"],
            "gas": TRANSACTION_CONFIG["GAS_LIMIT"],
        }

    def _create_blob_data(self) -> bytes:
        """Create padded blob data with encoded test string"""
        text = "<( o.O )>"
        encoded_text = abi.encode(["string"], [text])

        # Calculate padding to reach standard blob size
        padding_size = BLOB_CHUNK_SIZE * (BLOB_SIZE - len(encoded_text) // BLOB_CHUNK_SIZE)
        return (b"\x00" * padding_size) + encoded_text

    def _sign_with_blob_data(self, transaction: dict, account):
        """Sign transaction with attached blob data"""
        blob_data = self._create_blob_data()
        return account.sign_transaction(transaction, blobs=[blob_data])

    def _assert_transaction_rejected(self, web3: Web3, signed_transaction):
        """Verify Alpen EVM rejects blob transaction with expected error"""
        try:
            web3.eth.send_raw_transaction(signed_transaction.raw_transaction)
            raise AssertionError("Alpen EVM should reject EIP-4844 blob transactions")
        except Web3RPCError as e:
            actual_error = str(e.args[0])
            assert actual_error == EXPECTED_ERROR, (
                f"Expected Alpen EVM rejection: {EXPECTED_ERROR}, got: {actual_error}"
            )
