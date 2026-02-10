"""
EVM account management for functional tests.

Provides thread-safe nonce management and transaction signing utilities.
Based on the patterns from functional-tests/utils/evm_account.py.
"""

import threading
from typing import Optional

from eth_account import Account
from eth_account.signers.local import LocalAccount

from .config.constants import (
    DEV_CHAIN_ID,
    DEV_PRIVATE_KEY,
    DEV_RECIPIENT_ADDRESS,
    DEV_RECIPIENT_PRIVATE_KEY,
)


class ManagedAccount:
    """
    EVM account with thread-safe nonce management.

    This class wraps an eth_account Account and provides:
    - Thread-safe nonce tracking to avoid race conditions
    - Transaction signing utilities
    - Optional RPC-based nonce synchronization

    Usage:
        account = ManagedAccount.from_key(DEV_PRIVATE_KEY)
        raw_tx = account.sign_transfer(to=recipient, value=1000, gas_price=1000000000)
    """

    def __init__(self, account: LocalAccount, chain_id: int = DEV_CHAIN_ID):
        self._account = account
        self._chain_id = chain_id
        self._nonce: int = 0
        self._nonce_lock = threading.Lock()

    @classmethod
    def from_key(cls, private_key: str, chain_id: int = DEV_CHAIN_ID) -> "ManagedAccount":
        """Create a managed account from a private key."""
        account = Account.from_key(private_key)
        return cls(account, chain_id)

    @property
    def address(self) -> str:
        """The checksummed address of this account."""
        return self._account.address

    @property
    def private_key(self) -> str:
        """The private key of this account (hex string with 0x prefix)."""
        return self._account.key.hex()

    def get_nonce(self) -> int:
        """
        Get the next nonce and increment the internal counter.

        Thread-safe: multiple threads can safely call this method.
        """
        with self._nonce_lock:
            nonce = self._nonce
            self._nonce += 1
            return nonce

    def sync_nonce(self, nonce: int) -> None:
        """
        Synchronize the internal nonce counter with an external value.

        Use this after fetching the current nonce from an RPC node,
        or to reset after a failed transaction.
        """
        with self._nonce_lock:
            self._nonce = nonce

    def sign_transfer(
        self,
        *,
        to: str,
        value: int,
        gas_price: int,
        gas: int = 21000,
        nonce: Optional[int] = None,
    ) -> str:
        """
        Sign a simple ETH transfer transaction.

        Args:
            to: Recipient address
            value: Amount in wei
            gas_price: Gas price in wei
            gas: Gas limit (default 21000 for simple transfers)
            nonce: Optional nonce override. If not provided, uses internal counter.

        Returns:
            Raw signed transaction hex string (with 0x prefix)
        """
        if nonce is None:
            nonce = self.get_nonce()

        tx = {
            "nonce": nonce,
            "gasPrice": gas_price,
            "gas": gas,
            "to": to,
            "value": value,
            "data": b"",
            "chainId": self._chain_id,
        }
        signed = self._account.sign_transaction(tx)
        return "0x" + signed.raw_transaction.hex()

    def sign_transaction(
        self,
        *,
        to: str,
        value: int = 0,
        data: bytes = b"",
        gas_price: int,
        gas: int,
        nonce: Optional[int] = None,
    ) -> str:
        """
        Sign a general transaction (can include contract calls).

        Args:
            to: Recipient/contract address
            value: Amount in wei (default 0)
            data: Transaction data (default empty)
            gas_price: Gas price in wei
            gas: Gas limit
            nonce: Optional nonce override. If not provided, uses internal counter.

        Returns:
            Raw signed transaction hex string (with 0x prefix)
        """
        if nonce is None:
            nonce = self.get_nonce()

        tx = {
            "nonce": nonce,
            "gasPrice": gas_price,
            "gas": gas,
            "to": to,
            "value": value,
            "data": data,
            "chainId": self._chain_id,
        }
        signed = self._account.sign_transaction(tx)
        return "0x" + signed.raw_transaction.hex()


# =============================================================================
# Pre-configured Dev Accounts
# =============================================================================


def get_dev_account() -> ManagedAccount:
    """
    Create a new instance of the primary dev account (Foundry/Hardhat account #0).

    This account is pre-funded in dev chain configurations.

    IMPORTANT: Each call creates a fresh instance with its own nonce counter.
    This ensures test isolation - each test manages its own nonce state.
    Always call sync_nonce() after getting the account to sync with chain state.
    """
    return ManagedAccount.from_key(DEV_PRIVATE_KEY)


def get_recipient_account() -> ManagedAccount:
    """
    Create a new instance of the secondary dev account (Foundry/Hardhat account #1).

    Useful as a recipient address for transfer tests.

    IMPORTANT: Each call creates a fresh instance with its own nonce counter.
    """
    return ManagedAccount.from_key(DEV_RECIPIENT_PRIVATE_KEY)


# Convenient access to recipient address without needing the full account
RECIPIENT_ADDRESS = DEV_RECIPIENT_ADDRESS
