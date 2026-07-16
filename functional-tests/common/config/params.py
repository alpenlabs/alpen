"""
Rollup consensus parameters dataclasses.
"""

import json
from dataclasses import asdict, dataclass, field

from bitcoinlib.keys import Key


def hex_bytes_repeated(n: int, repeat: int = 32) -> str:
    """Generate hex string of repeated byte value.

    Args:
        n: Byte value (0-255)
        repeat: Number of times to repeat the byte

    Returns:
        Hex string representation

    Raises:
        ValueError: If n is not in valid byte range (0-255)
    """
    if not 0 <= n < 256:
        raise ValueError(f"Byte value must be in range 0-255, got: {n}")
    return bytes([n] * repeat).hex()


@dataclass
class L1BlockCommitment:
    height: int = field(default=100)
    # TODO(STR-3692): more type safe
    blkid: str = field(default_factory=lambda: hex_bytes_repeated(0))

    @staticmethod
    def at_latest_block(btc_rpc) -> "L1BlockCommitment":
        """Build an L1BlockCommitment from the current chain tip."""
        blkid = btc_rpc.proxy.getbestblockhash()
        blk_info = btc_rpc.proxy.getblock(blkid, 1)
        return L1BlockCommitment(blk_info["height"], blkid)


# TODO(STR-3692): move this to some place common as this should be useful for other purposes as well
def gen_random_keypair() -> tuple[str, Key]:
    """Generates a keypair and returns a tuple of xonly pubkey and privkey."""
    key = Key()
    xpubkey = format(key.x, "064x")
    return xpubkey, key


@dataclass
class GenesisAccountData:
    """Genesis snark account data. Maps to Rust GenesisSnarkAccountData."""

    predicate: str = (
        "Bip340Schnorr:4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d0766"
    )
    inner_state: str = field(default_factory=lambda: hex_bytes_repeated(0))
    balance: int = 0


@dataclass
class BridgeParams:
    """Bridge parameters. Maps to Rust BridgeParams."""

    denomination: int = 100_000_000
    max_withdrawal_amount: int | None = 1_000_000_000
    max_withdrawal_descriptor_len: int = 81


@dataclass
class OLParams:
    """OL genesis parameters. Maps to Rust OLParams."""

    accounts: dict[str, GenesisAccountData] = field(default_factory=dict)
    last_l1_block: L1BlockCommitment = field(default_factory=L1BlockCommitment)
    bridge_params: BridgeParams = field(default_factory=BridgeParams)

    def with_genesis_l1(self, genesis_l1_block: L1BlockCommitment) -> "OLParams":
        self.last_l1_block = genesis_l1_block
        return self

    def as_json_string(self) -> str:
        d = {
            "accounts": {k: asdict(v) for k, v in self.accounts.items()},
            "last_l1_block": asdict(self.last_l1_block),
            "bridge_params": asdict(self.bridge_params),
        }
        return json.dumps(d, indent=2)


@dataclass
class DepositTxParams:
    magic_bytes: list[int] = field(default_factory=lambda: [0, 0, 0, 0])
    deposit_amount: int = field(default=100000)
    address: str = field(default="")
    operators_pubkey: str = field(default="")
