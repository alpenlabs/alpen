"""
Shared EVM utilities for functional tests.

This module provides common helpers for EVM transaction testing,
including balance queries, transaction receipts, and account funding.
"""

from eth_account import Account

from .accounts import ManagedAccount
from .wait import wait_until


def get_balance(rpc, address: str, block_tag: str = "latest") -> int:
    """
    Get the balance of an address in wei.

    Args:
        rpc: RPC client instance
        address: The address to query
        block_tag: Block tag ("latest", "pending", "earliest") or hex block number

    Returns:
        Balance in wei as an integer
    """
    result = rpc.eth_getBalance(address, block_tag)
    return int(result, 16)


def wait_for_receipt(rpc, tx_hash: str, timeout: int = 30) -> dict:
    """
    Wait for a transaction receipt to be available.

    This polls the RPC until the transaction is mined or the timeout is reached.

    Args:
        rpc: RPC client instance
        tx_hash: Transaction hash to wait for
        timeout: Maximum time to wait in seconds

    Returns:
        The transaction receipt dict

    Raises:
        TimeoutError: If the transaction is not mined within the timeout
    """
    receipt: dict | None = None

    def check_receipt() -> bool:
        nonlocal receipt
        try:
            receipt = rpc.eth_getTransactionReceipt(tx_hash)
            return receipt is not None
        except Exception:
            return False

    wait_until(check_receipt, error_with=f"Transaction {tx_hash} not mined", timeout=timeout)
    assert receipt is not None  # For type checker
    return receipt


def create_funded_account(
    rpc,
    funding_account: ManagedAccount,
    amount: int,
    gas: int = 25000,
) -> ManagedAccount:
    """
    Create a new random account and fund it from an existing account.

    This is useful for test isolation - each test can create its own funded
    account to avoid nonce conflicts with other tests.

    Args:
        rpc: RPC client instance
        funding_account: Account to fund from (must have sufficient balance)
        amount: Amount to fund in wei
        gas: Gas limit for the funding transaction

    Returns:
        A new ManagedAccount with the specified balance
    """
    # Create new random account
    new_acct = Account.create()
    new_managed = ManagedAccount(new_acct)

    # Get gas price and send funding transaction
    gas_price = int(rpc.eth_gasPrice(), 16)
    raw_tx = funding_account.sign_transfer(
        to=new_acct.address,
        value=amount,
        gas_price=gas_price,
        gas=gas,
    )
    tx_hash = rpc.eth_sendRawTransaction(raw_tx)
    wait_for_receipt(rpc, tx_hash)

    return new_managed
