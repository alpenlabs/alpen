"""
EVM account management for functional tests.

Provides thread-safe nonce management and transaction signing utilities.
Based on the patterns from functional-tests/utils/evm_account.py.

Thread Safety:
    - ManagedAccount uses instance-level locks for general use
    - Dev accounts (get_dev_account, get_recipient_account) use module-level
      shared state to prevent nonce conflicts when tests run in parallel
"""

import threading

from eth_account import Account
from eth_account.signers.local import LocalAccount

from .config.constants import (
    DEV_CHAIN_ID,
    DEV_PRIVATE_KEY,
    DEV_RECIPIENT_ADDRESS,
    DEV_RECIPIENT_PRIVATE_KEY,
)

# =============================================================================
# Module-level shared state for dev accounts
# =============================================================================
# These locks and nonces are shared across all instances of dev accounts
# to prevent nonce conflicts when multiple tests use the same dev account
# (e.g., in parallel test execution).

_dev_account_lock = threading.Lock()
_dev_account_nonce: int = 0

_recipient_account_lock = threading.Lock()
_recipient_account_nonce: int = 0


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
        nonce: int | None = None,
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
        nonce: int | None = None,
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
# Shared Dev Account (Thread-Safe for Parallel Tests)
# =============================================================================


class _SharedDevAccount(ManagedAccount):
    """
    Dev account that uses module-level shared state for thread safety.

    This class overrides get_nonce and sync_nonce to use module-level
    locks and nonce counters, ensuring that parallel tests don't get
    conflicting nonces even when they each call get_dev_account().
    """

    def __init__(
        self,
        account: LocalAccount,
        shared_lock: threading.Lock,
        chain_id: int = DEV_CHAIN_ID,
    ):
        super().__init__(account, chain_id)
        self._shared_lock = shared_lock

    def get_nonce(self) -> int:
        """
        Get the next nonce using the shared module-level counter.

        Thread-safe across all instances of this account.
        """
        global _dev_account_nonce, _recipient_account_nonce

        with self._shared_lock:
            if self._shared_lock is _dev_account_lock:
                nonce = _dev_account_nonce
                _dev_account_nonce += 1
            else:
                nonce = _recipient_account_nonce
                _recipient_account_nonce += 1
            return nonce

    def sync_nonce(self, nonce: int) -> None:
        """
        Synchronize the shared nonce counter with an external value.

        This updates the module-level nonce, affecting all instances.
        """
        global _dev_account_nonce, _recipient_account_nonce

        with self._shared_lock:
            if self._shared_lock is _dev_account_lock:
                _dev_account_nonce = nonce
            else:
                _recipient_account_nonce = nonce


# =============================================================================
# Pre-configured Dev Accounts
# =============================================================================


def get_dev_account() -> ManagedAccount:
    """
    Get the primary dev account (Foundry/Hardhat account #0).

    This account is pre-funded in dev chain configurations.

    Thread Safety:
        All instances returned by this function share the same nonce counter.
        This prevents nonce conflicts when tests run in parallel and each
        calls get_dev_account().

    Usage:
        dev_account = get_dev_account()
        # Sync with chain state before first use
        nonce = int(rpc.eth_getTransactionCount(DEV_ADDRESS, "pending"), 16)
        dev_account.sync_nonce(nonce)
    """
    account = Account.from_key(DEV_PRIVATE_KEY)
    return _SharedDevAccount(account, _dev_account_lock)


def get_recipient_account() -> ManagedAccount:
    """
    Get the secondary dev account (Foundry/Hardhat account #1).

    Useful as a recipient address for transfer tests.

    Thread Safety:
        All instances returned by this function share the same nonce counter.
    """
    account = Account.from_key(DEV_RECIPIENT_PRIVATE_KEY)
    return _SharedDevAccount(account, _recipient_account_lock)


# Convenient access to recipient address without needing the full account
RECIPIENT_ADDRESS = DEV_RECIPIENT_ADDRESS
