"""EVM utilities for functional tests."""

from eth_account import Account

from .accounts import ManagedAccount
from .wait import wait_until


def get_balance(rpc, address: str, block_tag: str = "latest") -> int:
    """Get the balance of an address in wei."""
    result = rpc.eth_getBalance(address, block_tag)
    return int(result, 16)


def wait_for_receipt(rpc, tx_hash: str, timeout: int = 30) -> dict:
    """Wait for a transaction receipt."""
    receipt: dict | None = None

    def check_receipt() -> bool:
        nonlocal receipt
        try:
            receipt = rpc.eth_getTransactionReceipt(tx_hash)
            return receipt is not None
        except Exception:
            return False

    wait_until(check_receipt, error_with=f"Transaction {tx_hash} not mined", timeout=timeout)
    assert receipt is not None
    return receipt


def create_funded_account(
    rpc,
    funding_account: ManagedAccount,
    amount: int,
    gas: int = 25000,
) -> ManagedAccount:
    """Create a new random account and fund it."""
    new_acct = Account.create()
    new_managed = ManagedAccount(new_acct)

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
