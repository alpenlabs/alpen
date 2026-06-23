"""CREATE2 helpers for exercising the Safe Singleton Factory."""

from eth_hash.auto import keccak

CREATE2_SALT = (1).to_bytes(32, "big")
TINY_RUNTIME = bytes.fromhex("602a60005260206000f3")
TINY_RUNTIME_HEX = "0x" + TINY_RUNTIME.hex()


def tiny_init_code() -> bytes:
    """Builds init code for a tiny contract with deterministic runtime."""
    prefix_len = 12
    runtime_len = len(TINY_RUNTIME)
    return (
        bytes(
            [
                0x60,
                runtime_len,
                0x60,
                prefix_len,
                0x60,
                0x00,
                0x39,
                0x60,
                runtime_len,
                0x60,
                0x00,
                0xF3,
            ]
        )
        + TINY_RUNTIME
    )


def address_bytes(address: str) -> bytes:
    """Converts a hex Ethereum address to raw bytes."""
    return bytes.fromhex(address.removeprefix("0x"))


def create2_address(factory_address: str, salt: bytes, init_code: bytes) -> str:
    """Computes the CREATE2 address for a factory, salt, and init code."""
    digest = keccak(b"\xff" + address_bytes(factory_address) + salt + keccak(init_code))
    return "0x" + digest[-20:].hex()
