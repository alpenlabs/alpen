"""
Reusable EVM transaction helpers for functional tests.

Provides builders for common L2 operations: ETH transfers, contract
deployments with storage writes, and large-bytecode deployments.
"""

from eth_account import Account

from common.accounts import DEV_ACCOUNT, DEV_CHAIN_ID, DEV_PRIVATE_KEY

# Convenience re-export so callers can do ``from common.evm import DEV_ACCOUNT_ADDRESS``.
DEV_ACCOUNT_ADDRESS = DEV_ACCOUNT.address


def send_eth_transfer(rpc, nonce: int, to_addr: str, value_wei: int) -> str:
    """Send a simple ETH transfer and return the tx hash."""
    from common.accounts import sign_transfer

    gas_price = int(rpc.eth_gasPrice(), 16)
    raw_tx = sign_transfer(to=to_addr, value=value_wei, nonce=nonce, gas_price=gas_price)
    return rpc.eth_sendRawTransaction(raw_tx)


def sign_deploy(rpc, *, nonce: int, data: bytes, gas: int) -> str:
    """Sign and broadcast a contract-creation transaction. Returns tx hash."""
    tx = {
        "nonce": nonce,
        "gasPrice": int(rpc.eth_gasPrice(), 16),
        "gas": gas,
        "to": None,
        "value": 0,
        "data": data,
        "chainId": DEV_CHAIN_ID,
    }
    signed = Account.sign_transaction(tx, DEV_PRIVATE_KEY)
    return rpc.eth_sendRawTransaction(signed.raw_transaction.hex())


def deploy_storage_filler(rpc, nonce: int, num_slots: int) -> str:
    """Deploy a contract that writes to ``num_slots`` storage slots.

    Creates init code: SSTORE(0,1), SSTORE(1,2), ..., SSTORE(n-1,n)
    followed by minimal runtime (RETURN 1 byte).
    """
    init_code = b""
    for i in range(num_slots):
        init_code += bytes([0x7F]) + (i + 1).to_bytes(32, "big")  # PUSH32 value
        init_code += bytes([0x7F]) + i.to_bytes(32, "big")  # PUSH32 key
        init_code += bytes([0x55])  # SSTORE

    # Minimal runtime (PUSH1 1, PUSH1 0, RETURN)
    init_code += bytes([0x60, 0x01, 0x60, 0x00, 0xF3])

    gas = 100_000 + num_slots * 25_000
    return sign_deploy(rpc, nonce=nonce, data=init_code, gas=gas)


def deploy_large_runtime_contract(rpc, nonce: int, runtime_size: int = 10_000) -> str:
    """Deploy a contract with a large, deterministic runtime bytecode.

    Uses CODECOPY to store ``runtime_size`` bytes of 0xFE as the
    contract's runtime code.  Identical ``runtime_size`` values always
    produce the same code hash, which is useful for deduplication tests.
    """
    runtime_code = bytes([0xFE]) * runtime_size

    # Init code layout (14 bytes):
    #   PUSH2 runtime_size   ; 61 XX XX  (3)
    #   PUSH1 14             ; 60 0E     (2)  <- code offset
    #   PUSH1 0              ; 60 00     (2)  <- dest offset
    #   CODECOPY             ; 39        (1)
    #   PUSH2 runtime_size   ; 61 XX XX  (3)
    #   PUSH1 0              ; 60 00     (2)
    #   RETURN               ; F3        (1)
    prefix_size = 14
    init_code = bytearray()
    init_code += bytes([0x61]) + runtime_size.to_bytes(2, "big")
    init_code += bytes([0x60, prefix_size])
    init_code += bytes([0x60, 0x00])
    init_code += bytes([0x39])
    init_code += bytes([0x61]) + runtime_size.to_bytes(2, "big")
    init_code += bytes([0x60, 0x00])
    init_code += bytes([0xF3])

    assert len(init_code) == prefix_size
    init_code += runtime_code

    # Gas: intrinsic + calldata + code-deposit + execution headroom
    gas = 100_000 + 216 * runtime_size
    return sign_deploy(rpc, nonce=nonce, data=bytes(init_code), gas=gas)
