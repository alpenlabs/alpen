"""EVM utilities for functional tests."""

import logging
import time

from eth_account import Account
from eth_hash.auto import keccak

from .accounts import ManagedAccount
from .rpc import RpcError
from .wait import timeout_for_expected_blocks, wait_until

logger = logging.getLogger(__name__)

DEFAULT_RECEIPT_WAIT_BLOCKS = 10


def get_balance(rpc, address: str, block_tag: str = "latest") -> int:
    """Get the balance of an address in wei."""
    result = rpc.eth_getBalance(address, block_tag)
    return int(result, 16)


def subject_hex_from_address(address: str) -> str:
    """Convert an EVM address into the corresponding 32-byte subject hex."""
    return "00" * 12 + address.removeprefix("0x").lower()


def wait_for_ee_balance(
    alpen_rpc,
    btc_rpc,
    miner_addr: str,
    address: str,
    expected_wei: int,
    timeout: int = 600,
) -> None:
    deadline = time.time() + timeout
    last_balance = 0
    while time.time() < deadline:
        btc_rpc.proxy.generatetoaddress(4, miner_addr)
        time.sleep(1)
        last_balance = get_balance(alpen_rpc, address)
        if last_balance >= expected_wei:
            break
    else:
        raise AssertionError(
            f"EE balance for {address} did not reach {expected_wei} wei; got {last_balance}"
        )

    if last_balance != expected_wei:
        raise AssertionError(
            f"EE balance for {address} overshot: got {last_balance}, expected {expected_wei}"
        )


def wait_for_receipt(
    rpc,
    tx_hash: str,
    timeout: int | None = None,
    expected_blocks: int = DEFAULT_RECEIPT_WAIT_BLOCKS,
) -> dict:
    """Wait for a transaction receipt."""
    receipt: dict | None = None

    if timeout is None:
        timeout = timeout_for_expected_blocks(expected_blocks)

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


def send_raw_transaction(rpc, raw_tx: str) -> str:
    """Send a raw transaction, handling the sequencer forwarding race.

    Fullnodes forward transactions to the sequencer via --sequencer-http
    before adding them to the local pool. A race exists where the sequencer
    includes the tx in a block and gossips it back before the fullnode's
    local pool insertion completes, causing an "already known" error.

    When this happens the tx was already processed, but the RPC error means
    no hash is returned. We derive it ourselves as keccak256(raw_tx_bytes),
    which is the standard Ethereum transaction hash definition.
    """
    try:
        return rpc.eth_sendRawTransaction(raw_tx)
    except RpcError as e:
        if "already known" not in str(e):
            raise
        tx_hash = "0x" + keccak(bytes.fromhex(raw_tx[2:])).hex()
        logger.info(f"Tx already known (sequencer forwarding race), hash: {tx_hash}")
        return tx_hash


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
    tx_hash = send_raw_transaction(rpc, raw_tx)
    wait_for_receipt(rpc, tx_hash)

    return new_managed
