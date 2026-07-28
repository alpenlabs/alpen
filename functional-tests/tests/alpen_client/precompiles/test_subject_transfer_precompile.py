"""Verify the inter-EE subject-transfer precompile emits its intent event."""

import logging

import flexitest
from eth_account import Account
from eth_hash.auto import keccak
from eth_utils import to_checksum_address

from common.base_test import BaseTest
from common.config.constants import DEV_CHAIN_ID, DEV_PRIVATE_KEY, ServiceType
from common.evm import DEV_ACCOUNT_ADDRESS
from common.precompile import PRECOMPILE_SUBJECT_TRANSFER_ADDRESS, wait_for_receipt
from common.services import AlpenClientService
from envconfigs.alpen_client import AlpenClientEnv

logger = logging.getLogger(__name__)

SATS_TO_WEI = 10**10
GAS_LIMIT = 200_000

TRANSFER_AMOUNT_SATS = 123_456_789
DEST_ACCOUNT_HEX = "22" * 32
DEST_SUBJECT_HEX = "33" * 32
TRANSFER_DATA_HEX = "aabbcc"

EVENT_SIGNATURE = (
    "SubjectTransferIntentEvent(uint64,bytes32,bytes32,bytes32,bytes)"
)
EVENT_TOPIC = "0x" + keccak(EVENT_SIGNATURE.encode()).hex()


def build_subject_transfer_tx(rpc, nonce: int) -> dict:
    """Build a subject-transfer precompile transaction."""
    gas_price = int(rpc.eth_gasPrice(), 16)
    calldata_hex = DEST_ACCOUNT_HEX + DEST_SUBJECT_HEX + TRANSFER_DATA_HEX
    return {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": GAS_LIMIT,
        "to": to_checksum_address(PRECOMPILE_SUBJECT_TRANSFER_ADDRESS),
        "value": TRANSFER_AMOUNT_SATS * SATS_TO_WEI,
        "data": bytes.fromhex(calldata_hex),
        "chainId": DEV_CHAIN_ID,
    }


def topic0(log: dict) -> str:
    """Return the first topic from a receipt log."""
    topics = log.get("topics", [])
    assert topics, f"expected log topics, got {log}"
    return topics[0].lower()


def abi_word(value: int) -> str:
    """Encode a small integer as one ABI word."""
    return value.to_bytes(32, "big").hex()


def source_subject_hex() -> str:
    """Return the expected source subject for the dev EVM account."""
    return "00" * 12 + DEV_ACCOUNT_ADDRESS.removeprefix("0x").lower()


def assert_subject_transfer_event(receipt: dict):
    """Assert the receipt contains the expected subject-transfer intent event."""
    logs = receipt["logs"]
    assert len(logs) == 1, f"expected one subject-transfer log, got {len(logs)}: {logs}"

    log = logs[0]
    assert log["address"].lower() == PRECOMPILE_SUBJECT_TRANSFER_ADDRESS.lower(), (
        f"unexpected event address: {log['address']}"
    )
    assert topic0(log) == EVENT_TOPIC.lower(), (
        f"unexpected event topic: got {topic0(log)}, expected {EVENT_TOPIC}"
    )

    data = log["data"].removeprefix("0x").lower()
    expected_head = (
        abi_word(TRANSFER_AMOUNT_SATS)
        + source_subject_hex()
        + DEST_ACCOUNT_HEX
        + DEST_SUBJECT_HEX
        + abi_word(5 * 32)
    )
    expected_tail = (
        abi_word(len(bytes.fromhex(TRANSFER_DATA_HEX)))
        + TRANSFER_DATA_HEX
        + "00" * (32 - len(bytes.fromhex(TRANSFER_DATA_HEX)))
    )
    assert data == expected_head + expected_tail, "subject-transfer event data mismatch"


@flexitest.register
class TestSubjectTransferPrecompile(BaseTest):
    """Subject-transfer precompile emits a structured intent event."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(AlpenClientEnv(fullnode_count=0, enable_l1_da=True))

    def main(self, ctx) -> bool:
        sequencer: AlpenClientService = self.get_service(ServiceType.AlpenSequencer)
        rpc = sequencer.create_rpc()

        nonce = int(rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
        tx = build_subject_transfer_tx(rpc, nonce)
        signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)

        logger.info("Calling subject-transfer precompile")
        tx_hash = rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())
        receipt = wait_for_receipt(rpc, tx_hash, timeout=30)

        assert receipt["status"] in (1, "0x1"), (
            f"subject-transfer precompile should succeed, got status {receipt['status']}"
        )
        assert_subject_transfer_event(receipt)

        logger.info("Subject-transfer precompile emitted the expected intent event")
        return True
