import hashlib

from strata_utils import sign_schnorr_sig
from web3 import Web3

from utils import wait_until_with_value


def get_schnorr_precompile_input(secret_key: str, msg: str) -> str:
    """
    Generates the strata schnorr precompile input by signing the SHA-256 hash of the message.

    Args:
        secret_key (str): The secret key used for signing.
        msg (str): The message to sign.

    Returns:
        str: Schnorr precompile input.
    """
    msg_bytes = msg.encode("utf-8")
    message_hash = hashlib.sha256(msg_bytes).hexdigest()
    signature, public_key = sign_schnorr_sig(message_hash, secret_key)

    return public_key.hex() + message_hash + signature.hex()


def make_precompile_call(
    web3: Web3, precompile_address: str, precompile_input: str
) -> tuple[str, str]:
    """
    Executes a Schnorr precompile call.

    Args:
        web3 (Web3): An instance of Web3.
        precompile_address (str): The address of the  precompile.
        precompile_input (str): The input data for the precompile.

    Returns:
        Tuple[str, str]: A tuple containing the transaction hash and result of precompile call.

    Raises:
        ConnectionError: If unable to connect to the blockchain.
        RuntimeError: If the transaction fails.
    """
    if not web3.is_connected():
        raise ConnectionError("Cannot connect to reth")

    source = web3.address

    # Simulate the precompile call (safe because precompile is stateless)
    simulated_result = web3.eth.call(
        {
            "to": precompile_address,
            "data": precompile_input,
        }
    )

    tx_params = {
        "to": precompile_address,
        "from": source,
        "value": hex(0),
        "gas": hex(200000),
        "data": precompile_input,
    }
    txid = web3.eth.send_transaction(tx_params)

    receipt = wait_until_with_value(
        lambda: web3.eth.get_transaction_receipt(txid),
        lambda result: not isinstance(result, Exception),
        error_with="Transaction receipt for txid not available",
    )

    if receipt.status != 1:
        raise RuntimeError("Precompile transaction failed")

    return txid, simulated_result.to_0x_hex()


def get_test_schnnor_secret_key() -> str:
    """Return the test Schnnor secret key."""
    return "a9f913c3d7fe56c462228ad22bb7631742a121a6a138d57c1fc4a351314948fa"
