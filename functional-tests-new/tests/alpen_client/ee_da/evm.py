"""
EVM transaction builders for DA pipeline testing.

Provides helpers to construct and submit L2 transactions that generate
state diffs of varying sizes — from simple ETH transfers to large
multi-slot contract deployments.
"""

# Dev account from alpen-dev-chain.json (standard Hardhat dev account)
DEV_ACCOUNT_ADDRESS = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
DEV_ACCOUNT_PRIVATE_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
DEV_CHAIN_ID = 2892


def send_eth_transfer(rpc, nonce: int, to_addr: str, value_wei: int) -> str:
    """Send an ETH transfer transaction. Returns tx hash."""
    from eth_account import Account

    tx = {
        "nonce": nonce,
        "gasPrice": int(rpc.eth_gasPrice(), 16),
        "gas": 21000,
        "to": to_addr,
        "value": value_wei,
        "data": b"",
        "chainId": DEV_CHAIN_ID,
    }
    signed = Account.sign_transaction(tx, DEV_ACCOUNT_PRIVATE_KEY)
    return rpc.eth_sendRawTransaction(signed.raw_transaction.hex())


def deploy_storage_filler(rpc, nonce: int, num_slots: int) -> str:
    """
    Deploy a contract that writes to many storage slots.

    Creates init code that does: SSTORE(0, 1), SSTORE(1, 2), ..., SSTORE(n-1, n)
    This generates a large state diff for testing multi-chunk DA.
    """
    from eth_account import Account

    # Build init code: for each slot, PUSH32 value, PUSH32 key, SSTORE
    init_code = b""
    for i in range(num_slots):
        init_code += bytes([0x7F]) + (i + 1).to_bytes(32, "big")  # PUSH32 value
        init_code += bytes([0x7F]) + i.to_bytes(32, "big")  # PUSH32 key
        init_code += bytes([0x55])  # SSTORE

    # Return minimal runtime code (STOP)
    init_code += bytes([0x60, 0x01, 0x60, 0x00, 0xF3])  # PUSH1 1, PUSH1 0, RETURN

    tx = {
        "nonce": nonce,
        "gasPrice": int(rpc.eth_gasPrice(), 16),
        "gas": 100_000 + num_slots * 25_000,
        "to": None,
        "value": 0,
        "data": init_code,
        "chainId": DEV_CHAIN_ID,
    }
    signed = Account.sign_transaction(tx, DEV_ACCOUNT_PRIVATE_KEY)
    return rpc.eth_sendRawTransaction(signed.raw_transaction.hex())


def deploy_large_runtime_contract(rpc, nonce: int, runtime_size: int = 10_000) -> str:
    """
    Deploy a contract with a large, deterministic runtime bytecode.

    Uses CODECOPY to store ``runtime_size`` bytes of 0xFE as the contract's
    runtime code.  All calls with the same ``runtime_size`` produce identical
    runtime bytecodes (and therefore the same code hash), enabling cross-batch
    bytecode deduplication testing.
    """
    from eth_account import Account

    # Runtime code: padding bytes (0xFE = INVALID opcode, harmless as stored code)
    runtime_code = bytes([0xFE]) * runtime_size

    # Init code layout (14 bytes):
    #   PUSH2 runtime_size   ; 61 XX XX  (3)
    #   PUSH1 14             ; 60 0E     (2)  <- code offset in initcode
    #   PUSH1 0              ; 60 00     (2)  <- dest offset in memory
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

    # Gas breakdown:
    #   53,000  = intrinsic (21k base + 32k create)
    #   16 * (prefix_size + runtime_size) = calldata cost (non-zero bytes)
    #   200 * runtime_size = code deposit cost (EIP-3541)
    #   ~5,000  = execution (CODECOPY, memory expansion, etc.)
    gas = 100_000 + 216 * runtime_size

    tx = {
        "nonce": nonce,
        "gasPrice": int(rpc.eth_gasPrice(), 16),
        "gas": gas,
        "to": None,
        "value": 0,
        "data": bytes(init_code),
        "chainId": DEV_CHAIN_ID,
    }
    signed = Account.sign_transaction(tx, DEV_ACCOUNT_PRIVATE_KEY)
    return rpc.eth_sendRawTransaction(signed.raw_transaction.hex())
