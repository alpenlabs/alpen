"""
Python wrapper for strata-test-cli binary.

Provides a clean Python interface to the Rust test CLI for functional tests,
replacing the previous PyO3 FFI bindings with subprocess calls to the binary.
"""

import json

from utils.utils import run_tty

BINARY_PATH = "strata-test-cli"


def _run_command(args: list[str]) -> str:
    """
    Run a CLI command and return stdout.

    Args:
        args: Command arguments to pass to strata-test-cli

    Returns:
        Command stdout as string

    Raises:
        subprocess.CalledProcessError: If command fails
    """
    cmd = [BINARY_PATH] + args
    result = run_tty(cmd, capture_output=True)
    result.check_returncode()
    return result.stdout.decode("utf-8").strip()


def create_deposit_transaction(
    drt_tx: bytes,
    operator_keys: list[bytes],
    index: int,
) -> bytes:
    """
    Create a deposit transaction from DRT.

    Args:
        drt_tx: Raw DRT transaction bytes
        operator_keys: List of operator private keys (each 78 bytes)
        index: Deposit transaction index

    Returns:
        Signed deposit transaction as bytes
    """
    drt_tx_hex = drt_tx.hex()
    operator_keys_json = json.dumps([key.hex() for key in operator_keys])

    # fmt: off
    args = [
        "create-deposit-tx",
        "--drt-tx", drt_tx_hex,
        "--operator-keys", operator_keys_json,
        "--index", str(index),
    ]
    # fmt: on

    result_hex = _run_command(args)
    return bytes.fromhex(result_hex)


def create_withdrawal_fulfillment(
    destination: str,
    amount: int,
    operator_idx: int,
    deposit_idx: int,
    deposit_txid: str,
    btc_url: str,
    btc_user: str,
    btc_password: str,
) -> bytes:
    """
    Create a withdrawal fulfillment transaction.

    Args:
        destination: Destination Bitcoin address (BOSD format)
        amount: Amount in satoshis
        operator_idx: Operator index
        deposit_idx: Deposit index
        deposit_txid: Deposit transaction ID (hex)
        btc_url: Bitcoin RPC URL
        btc_user: Bitcoin RPC username
        btc_password: Bitcoin RPC password

    Returns:
        Withdrawal fulfillment transaction as bytes
    """
    # fmt: off
    args = [
        "create-withdrawal-fulfillment",
        "--destination", destination,
        "--amount", str(amount),
        "--operator-idx", str(operator_idx),
        "--deposit-idx", str(deposit_idx),
        "--deposit-txid", deposit_txid,
        "--btc-url", btc_url,
        "--btc-user", btc_user,
        "--btc-password", btc_password,
    ]
    # fmt: on

    result_hex = _run_command(args)
    return bytes.fromhex(result_hex)


def get_address(index: int) -> str:
    """
    Get a taproot address at a specific index.

    Args:
        index: Address index

    Returns:
        Taproot address as string
    """
    # fmt: off
    args = [
        "get-address",
        "--index", str(index),
    ]
    # fmt: on

    return _run_command(args)


def musig_aggregate_pks(pubkeys: list[str]) -> str:
    """
    Aggregate public keys using MuSig2.

    Args:
        pubkeys: List of X-only public keys (hex strings)

    Returns:
        Aggregated public key as hex string
    """
    pubkeys_json = json.dumps(pubkeys)

    # fmt: off
    args = [
        "musig-aggregate-pks",
        "--pubkeys", pubkeys_json,
    ]
    # fmt: on

    return _run_command(args)


def extract_p2tr_pubkey(address: str) -> str:
    """
    Extract P2TR public key from a taproot address.

    Args:
        address: Taproot address

    Returns:
        X-only public key as hex string
    """
    # fmt: off
    args = [
        "extract-p2tr-pubkey",
        "--address", address,
    ]
    # fmt: on

    return _run_command(args)


def convert_to_xonly_pk(pubkey: str) -> str:
    """
    Convert a public key to X-only format.

    Args:
        pubkey: Public key in hex-encoded string

    Returns:
        X-only public key as hex string
    """
    # fmt: off
    args = [
        "convert-to-xonly-pk",
        "--pubkey", pubkey,
    ]
    # fmt: on

    return _run_command(args)


def sign_schnorr_sig(message: str, secret_key: str) -> tuple[bytes, bytes]:
    """
    Sign a message using Schnorr signature.

    Args:
        message: Message hash in hex-encoded string
        secret_key: Secret key in hex-encoded string

    Returns:
        Tuple of (signature bytes, public key bytes)
    """
    # fmt: off
    args = [
        "sign-schnorr-sig",
        "--message", message,
        "--secret-key", secret_key,
    ]
    # fmt: on

    result_json = _run_command(args)
    result = json.loads(result_json)

    signature = bytes.fromhex(result["signature"])
    public_key = bytes.fromhex(result["public_key"])

    return (signature, public_key)


def xonlypk_to_descriptor(xonly_pubkey: str) -> str:
    """
    Convert X-only public key to BOSD descriptor.

    Args:
        xonly_pubkey: X-only public key in hex-encoded string

    Returns:
        BOSD descriptor as string
    """
    # fmt: off
    args = [
        "xonlypk-to-descriptor",
        "--xonly-pubkey", xonly_pubkey,
    ]
    # fmt: on

    return _run_command(args)
