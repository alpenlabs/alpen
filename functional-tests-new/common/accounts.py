"""
Standard Ethereum dev accounts for testing.

These are the standard Foundry/Hardhat dev accounts with known private keys.
They are pre-funded in dev chain configurations.
"""

from eth_account import Account

# Standard foundry/hardhat dev account #0
DEV_PRIVATE_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
DEV_ACCOUNT = Account.from_key(DEV_PRIVATE_KEY)

# Standard foundry/hardhat dev account #1
RECIPIENT_PRIVATE_KEY = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
RECIPIENT_ACCOUNT = Account.from_key(RECIPIENT_PRIVATE_KEY)

# Chain ID for alpen-dev-chain
DEV_CHAIN_ID = 2892


def sign_transfer(
    *,
    to: str,
    value: int,
    nonce: int,
    gas_price: int,
    gas: int = 21000,
    chain_id: int = DEV_CHAIN_ID,
    private_key: str = DEV_PRIVATE_KEY,
) -> str:
    """Sign a simple ETH transfer transaction. Returns raw tx hex."""
    tx = {
        "nonce": nonce,
        "gasPrice": gas_price,
        "gas": gas,
        "to": to,
        "value": value,
        "data": b"",
        "chainId": chain_id,
    }
    signed = Account.sign_transaction(tx, private_key)
    return "0x" + signed.raw_transaction.hex()
