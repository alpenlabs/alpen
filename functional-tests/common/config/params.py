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
    blkid: str = field(default_factory=lambda: hex_bytes_repeated(0))  # TODO: more type safe


@dataclass
class GenesisL1View:
    blk: L1BlockCommitment = field(default_factory=L1BlockCommitment)
    next_target: int = field(default=1000)
    epoch_start_timestamp: int = field(default=1000)
    last_11_timestamps: list[int] = field(default_factory=lambda: [0] * 11)  # TODO: more type safe

    @staticmethod
    def at_latest_block(btc_rpc) -> "GenesisL1View":
        """Build a GenesisL1View from the current chain tip.

        Fetches the real target, epoch start timestamp, and last 11 block
        timestamps from bitcoind so that the ASM's HeaderVerificationState
        can correctly validate subsequent block headers.
        """
        blkid = btc_rpc.proxy.getbestblockhash()
        blk_info = btc_rpc.proxy.getblock(blkid, 1)
        blkheight = blk_info["height"]
        l1blk_commitment = L1BlockCommitment(blkheight, blkid)

        # next_target: compact target (nBits) from the current block header.
        # In regtest, difficulty is constant so this stays the same.
        # The "bits" field in getblock returns a hex string of the compact target.
        next_target = int(blk_info["bits"], 16)

        # epoch_start_timestamp: timestamp of the most recent difficulty
        # adjustment block. difficulty_adjustment_interval = 2016 for all
        # networks (including regtest).
        difficulty_interval = 2016
        epoch_start_height = (blkheight // difficulty_interval) * difficulty_interval
        epoch_start_hash = btc_rpc.proxy.getblockhash(epoch_start_height)
        epoch_start_info = btc_rpc.proxy.getblock(epoch_start_hash, 1)
        epoch_start_timestamp = epoch_start_info["time"]

        # last_11_timestamps: timestamps of the last 11 blocks in ascending
        # order (oldest first). If chain is shorter than 11 blocks, pad with 0.
        timestamps = []
        for i in range(11):
            h = blkheight - (10 - i)
            if h < 0:
                timestamps.append(0)
            else:
                h_hash = btc_rpc.proxy.getblockhash(h)
                h_info = btc_rpc.proxy.getblock(h_hash, 1)
                timestamps.append(h_info["time"])

        return GenesisL1View(
            blk=l1blk_commitment,
            next_target=next_target,
            epoch_start_timestamp=epoch_start_timestamp,
            last_11_timestamps=timestamps,
        )


# TODO: move this to some place common as this should be useful for other purposes as well
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
class OLParams:
    """OL genesis parameters. Maps to Rust OLParams."""

    accounts: dict[str, GenesisAccountData] = field(default_factory=dict)
    last_l1_block: L1BlockCommitment = field(default_factory=L1BlockCommitment)

    def with_genesis_l1(self, genesis_l1: GenesisL1View) -> "OLParams":
        self.last_l1_block = genesis_l1.blk
        return self

    def as_json_string(self) -> str:
        d = {
            "accounts": {k: asdict(v) for k, v in self.accounts.items()},
            "last_l1_block": asdict(self.last_l1_block),
        }
        return json.dumps(d, indent=2)


@dataclass
class DepositTxParams:
    magic_bytes: list[int] = field(default_factory=lambda: [0, 0, 0, 0])
    deposit_amount: int = field(default=100000)
    address: str = field(default="")
    operators_pubkey: str = field(default="")
