"""EVM account management with per-instance nonce tracking."""

import threading

from eth_account import Account
from eth_account.signers.local import LocalAccount

from .config.constants import (
    DEV_CHAIN_ID,
    DEV_PRIVATE_KEY,
    DEV_RECIPIENT_ADDRESS,
    DEV_RECIPIENT_PRIVATE_KEY,
)


class ManagedAccount:
    """EVM account with thread-safe nonce management.

    Each instance maintains its own nonce counter. Use ``sync_nonce`` to
    initialize it from chain state, then ``get_nonce`` / ``sign_transfer``
    to auto-increment.
    """

    def __init__(self, account: LocalAccount, chain_id: int = DEV_CHAIN_ID):
        self._account = account
        self._chain_id = chain_id
        self._nonce: int = 0
        self._nonce_lock = threading.Lock()

    @classmethod
    def from_key(cls, private_key: str, chain_id: int = DEV_CHAIN_ID) -> "ManagedAccount":
        account = Account.from_key(private_key)
        return cls(account, chain_id)

    @property
    def address(self) -> str:
        return self._account.address

    @property
    def private_key(self) -> str:
        return self._account.key.hex()

    def get_nonce(self) -> int:
        """Get the next nonce and increment the internal counter."""
        with self._nonce_lock:
            nonce = self._nonce
            self._nonce += 1
            return nonce

    def sync_nonce(self, nonce: int) -> None:
        """Sync the internal nonce counter with chain state."""
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
        """Sign a simple ETH transfer. Returns raw tx hex."""
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
        """Sign a general transaction. Returns raw tx hex."""
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


def get_dev_account(rpc) -> ManagedAccount:
    """Get the primary dev account with nonce synced from chain.

    Returns a fresh instance whose nonce counter is initialized from
    the pending on-chain nonce. No module-level state is shared, so
    different tests and environments cannot interfere with each other.
    """
    account = ManagedAccount.from_key(DEV_PRIVATE_KEY)
    nonce = int(rpc.eth_getTransactionCount(account.address, "pending"), 16)
    account.sync_nonce(nonce)
    return account


def get_recipient_account() -> ManagedAccount:
    """Get the secondary dev account (Foundry/Hardhat account #1)."""
    return ManagedAccount.from_key(DEV_RECIPIENT_PRIVATE_KEY)


RECIPIENT_ADDRESS = DEV_RECIPIENT_ADDRESS
