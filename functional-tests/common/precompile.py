"""
Precompile call helpers and address constants.

Provides utilities for calling EVM precompiles via JSON-RPC, including
both simulation (eth_call) and on-chain transaction submission. Also
includes Schnorr signature helpers backed by the strata-test-cli binary.
"""

import hashlib
import json
import logging
import subprocess

from eth_account import Account
from eth_utils import to_checksum_address

from common.config.constants import DEV_CHAIN_ID, DEV_PRIVATE_KEY
from common.evm import DEV_ACCOUNT_ADDRESS
from common.rpc import RpcError
from common.wait import wait_until_with_value

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Alpen custom precompile addresses
# ---------------------------------------------------------------------------
PRECOMPILE_BRIDGEOUT_ADDRESS = "0x5400000000000000000000000000000000000001"
PRECOMPILE_SCHNORR_ADDRESS = "0x5400000000000000000000000000000000000002"

# ---------------------------------------------------------------------------
# Standard EVM precompile addresses (0x01 - 0x08)
# ---------------------------------------------------------------------------
PRECOMPILE_ECRECOVER = "0x0000000000000000000000000000000000000001"
PRECOMPILE_SHA256 = "0x0000000000000000000000000000000000000002"
PRECOMPILE_RIPEMD160 = "0x0000000000000000000000000000000000000003"
PRECOMPILE_IDENTITY = "0x0000000000000000000000000000000000000004"
PRECOMPILE_MODEXP = "0x0000000000000000000000000000000000000005"
PRECOMPILE_ECADD = "0x0000000000000000000000000000000000000006"
PRECOMPILE_ECMUL = "0x0000000000000000000000000000000000000007"
PRECOMPILE_ECPAIRING = "0x0000000000000000000000000000000000000008"

# ---------------------------------------------------------------------------
# EIP-4844 point evaluation
# ---------------------------------------------------------------------------
PRECOMPILE_POINT_EVALUATION = "0x000000000000000000000000000000000000000a"

# ---------------------------------------------------------------------------
# BLS12-381 precompile addresses (EIP-2537, 0x0b - 0x11)
# ---------------------------------------------------------------------------
PRECOMPILE_BLS12_G1ADD = "0x000000000000000000000000000000000000000b"
PRECOMPILE_BLS12_G1MSM = "0x000000000000000000000000000000000000000c"
PRECOMPILE_BLS12_G2ADD = "0x000000000000000000000000000000000000000d"
PRECOMPILE_BLS12_G2MSM = "0x000000000000000000000000000000000000000e"
PRECOMPILE_BLS12_PAIRING_CHECK = "0x000000000000000000000000000000000000000f"
PRECOMPILE_BLS12_MAP_FP_TO_G1 = "0x0000000000000000000000000000000000000010"
PRECOMPILE_BLS12_MAP_FP2_TO_G2 = "0x0000000000000000000000000000000000000011"


# ---------------------------------------------------------------------------
# Precompile call helpers
# ---------------------------------------------------------------------------


def _ensure_0x(hex_str: str) -> str:
    """Ensure a hex string has the 0x prefix."""
    return hex_str if hex_str.startswith("0x") else "0x" + hex_str


def normalize_hex(h: str) -> str:
    """Strip 0x prefix and lowercase for comparison."""
    return h.lower().removeprefix("0x")


def eth_call_precompile(rpc, address: str, input_hex: str) -> str:
    """Simulate a precompile call via eth_call.

    Args:
        rpc: JsonRpcClient instance.
        address: Precompile address (0x-prefixed).
        input_hex: Call data as hex string (with or without 0x prefix).

    Returns:
        Hex-encoded result string from the RPC (0x-prefixed).
    """
    data = _ensure_0x(input_hex)
    return rpc.eth_call({"to": address, "data": data}, "latest")


def send_precompile_tx(
    rpc,
    address: str,
    input_hex: str,
    nonce: int,
    gas: int = 200_000,
) -> str:
    """Sign and send a precompile call as an on-chain transaction.

    Args:
        rpc: JsonRpcClient instance.
        address: Precompile address (0x-prefixed).
        input_hex: Call data as hex string (with or without 0x prefix).
        nonce: Sender nonce.
        gas: Gas limit (default 200000).

    Returns:
        Transaction hash as hex string.
    """
    data_hex = _ensure_0x(input_hex)
    gas_price = int(rpc.eth_gasPrice(), 16)
    tx = {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": gas,
        "to": to_checksum_address(address),
        "value": 0,
        "data": bytes.fromhex(data_hex[2:]),
        "chainId": DEV_CHAIN_ID,
    }
    signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)
    return rpc.eth_sendRawTransaction("0x" + signed.raw_transaction.hex())


def wait_for_receipt(rpc, tx_hash: str, timeout: int = 30) -> dict:
    """Poll eth_getTransactionReceipt until available.

    Args:
        rpc: JsonRpcClient instance.
        tx_hash: Transaction hash to look up.
        timeout: Maximum seconds to wait.

    Returns:
        Transaction receipt dict.

    Raises:
        AssertionError: If receipt is not available within timeout.
    """

    def _get():
        try:
            return rpc.eth_getTransactionReceipt(tx_hash)
        except RpcError:
            return None

    return wait_until_with_value(
        _get,
        lambda r: r is not None,
        error_with=f"Receipt for {tx_hash} not available within {timeout}s",
        timeout=timeout,
    )


def call_precompile(rpc, address: str, input_hex: str) -> tuple[str, str]:
    """Simulate a precompile call and submit the same call on-chain.

    Fetches the current nonce, sends a signed transaction, and waits for
    the receipt.  Equivalent to the old ``make_precompile_call()`` helper.

    Args:
        rpc: JsonRpcClient instance.
        address: Precompile address (0x-prefixed).
        input_hex: Call data as hex string (with or without 0x prefix).

    Returns:
        Tuple of (tx_hash, simulated_result_hex).

    Raises:
        RuntimeError: If the on-chain transaction reverts.
    """
    simulated = eth_call_precompile(rpc, address, input_hex)

    nonce = int(rpc.eth_getTransactionCount(DEV_ACCOUNT_ADDRESS, "latest"), 16)
    tx_hash = send_precompile_tx(rpc, address, input_hex, nonce)

    receipt = wait_for_receipt(rpc, tx_hash)
    status = receipt["status"]
    if isinstance(status, str):
        status = int(status, 16)
    if status != 1:
        raise RuntimeError(f"Precompile transaction reverted: {tx_hash}")

    return tx_hash, simulated


# ---------------------------------------------------------------------------
# Schnorr signature helpers (uses strata-test-cli binary)
# ---------------------------------------------------------------------------

SCHNORR_TEST_SECRET_KEY = "a9f913c3d7fe56c462228ad22bb7631742a121a6a138d57c1fc4a351314948fa"


def sign_schnorr_sig(message_hash: str, secret_key: str) -> tuple[bytes, bytes]:
    """Sign a message hash using Schnorr via strata-test-cli.

    Args:
        message_hash: Hex-encoded SHA-256 hash of the message (no 0x prefix).
        secret_key: Hex-encoded secret key (no 0x prefix).

    Returns:
        Tuple of (signature_bytes, public_key_bytes).
    """
    result = subprocess.run(
        [
            "strata-test-cli",
            "sign-schnorr-sig",
            "--message",
            message_hash,
            "--secret-key",
            secret_key,
        ],
        capture_output=True,
        check=True,
        text=True,
    )
    data = json.loads(result.stdout.strip())
    signature = bytes.fromhex(data["signature"])
    public_key = bytes.fromhex(data["public_key"])
    return signature, public_key


def get_schnorr_precompile_input(secret_key: str, msg: str) -> str:
    """Generate Schnorr precompile call data from a message string.

    Computes SHA-256(msg), signs it with the given secret key, and returns
    the concatenation ``pubkey(32B) || msg_hash(32B) || signature(64B)`` as
    a hex string (no 0x prefix).

    Args:
        secret_key: Hex-encoded secret key (no 0x prefix).
        msg: Plaintext message to sign.

    Returns:
        Hex string of precompile input (no 0x prefix).
    """
    message_hash = hashlib.sha256(msg.encode("utf-8")).hexdigest()
    signature, public_key = sign_schnorr_sig(message_hash, secret_key)
    return public_key.hex() + message_hash + signature.hex()
