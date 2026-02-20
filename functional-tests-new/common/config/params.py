"""
Rollup consensus parameters dataclasses.
"""

import json
from dataclasses import asdict, dataclass, field
from typing import Literal

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
class GenesisHeaderParams:
    timestamp: int = field(default=0)
    slot: int = field(default=0)
    epoch: int = field(default=0)
    parent_blkid: str = field(default_factory=lambda: hex_bytes_repeated(0))
    body_root: str = field(default_factory=lambda: hex_bytes_repeated(0))
    logs_root: str = field(default_factory=lambda: hex_bytes_repeated(0))


@dataclass
class GenesisAccountData:
    predicate: str = field(default="AlwaysAccept")
    inner_state: str = field(default_factory=lambda: hex_bytes_repeated(0))
    balance: int = field(default=0)


@dataclass
class GenesisL1View:
    blk: L1BlockCommitment = field(default_factory=L1BlockCommitment)
    next_target: int = field(default=1000)
    epoch_start_timestamp: int = field(default=1000)
    last_11_timestamps: list[int] = field(default_factory=lambda: [0] * 11)  # TODO: more type safe

    @staticmethod
    def at_latest_block(btc_rpc) -> "GenesisL1View":
        blkid = btc_rpc.proxy.getbestblockhash()
        blkheight = btc_rpc.proxy.getblock(blkid, 1)["height"]
        l1blk_commitment = L1BlockCommitment(blkheight, blkid)
        # TODO: add timestamps as needed
        return GenesisL1View(l1blk_commitment)


# TODO: move this to some place common as this should be useful for other purposes as well
def gen_random_keypair() -> tuple[str, Key]:
    """Generates a keypair and returns a tuple of xonly pubkey and privkey."""
    key = Key()
    xpubkey = format(key.x, "064x")
    return xpubkey, key


def compressed_pubkey(key: Key) -> str:
    """Returns the 33-byte compressed public key hex (02/03 prefix + x)."""
    prefix = "02" if key.y % 2 == 0 else "03"
    return prefix + format(key.x, "064x")


def even_pubkey(key: Key) -> str:
    """Returns the 33-byte even-parity compressed public key hex (02 prefix + x)."""
    return "02" + format(key.x, "064x")


@dataclass
class ProofPublishModeTimeout:
    timeout: int = field(default=30)


ProofPublishMode = Literal["strict"] | ProofPublishModeTimeout


@dataclass
class SchnorrVerify:
    schnorr_key: str  # TODO: more sophisticated, using pydantic?


CredRule = SchnorrVerify | Literal["unchecked"]


@dataclass
class RollupParams:
    magic_bytes: str = "ALPN"
    block_time: int = field(default=5000)  # millisecs
    cred_rule: CredRule = field(default="unchecked")
    genesis_l1_view: GenesisL1View = field(default_factory=GenesisL1View)
    operators: list[str] = field(default_factory=lambda: [gen_random_keypair()[0]])
    evm_genesis_block_hash: str = field(default_factory=lambda: hex_bytes_repeated(0))
    evm_genesis_block_state_root: str = field(default_factory=lambda: hex_bytes_repeated(0))
    l1_reorg_safe_depth: int = field(default=6)
    target_l2_batch_size: int = field(default=64)
    deposit_amount: int = field(default=100000)
    recovery_delay: int = field(default=1008)
    checkpoint_predicate: str = field(default="AlwaysAccept")
    dispatch_assignment_dur: int = field(default=144)
    proof_publish_mode: ProofPublishMode = field(default_factory=ProofPublishModeTimeout)
    max_deposits_in_block: int = field(default=10)
    network: str = field(default="regtest")

    def as_json_string(self) -> str:
        d = asdict(self)
        return json.dumps(d, indent=2)

    def with_genesis_l1(self, genesis_l1: GenesisL1View):
        self.genesis_l1_view = genesis_l1
        return self


@dataclass
class OLParams:
    header: GenesisHeaderParams | None = field(default=None)
    accounts: dict[str, GenesisAccountData] = field(default_factory=dict)
    last_l1_block: L1BlockCommitment = field(default_factory=L1BlockCommitment)

    def as_json_string(self) -> str:
        d = asdict(self)
        if d.get("header") is None:
            d.pop("header", None)
        return json.dumps(d, indent=2)

    def with_genesis_l1(self, genesis_l1: GenesisL1View):
        self.last_l1_block = genesis_l1.blk
        return self


@dataclass
class ThresholdConfig:
    keys: list[str]
    threshold: int = 1


@dataclass
class AdminSubprotocolConfig:
    strata_administrator: ThresholdConfig
    strata_sequencer_manager: ThresholdConfig
    confirmation_depth: int = 144


@dataclass
class CheckpointSubprotocolConfig:
    sequencer_predicate: str = "AlwaysAccept"
    checkpoint_predicate: str = "AlwaysAccept"
    genesis_l1_height: int = 0
    genesis_ol_blkid: str = field(default_factory=lambda: hex_bytes_repeated(0))


@dataclass
class BridgeV1SubprotocolConfig:
    operators: list[str] = field(default_factory=list)
    denomination: int = field(default=1_000_000_000)
    assignment_duration: int = field(default=64)
    operator_fee: int = field(default=50_000_000)
    recovery_delay: int = field(default=1008)


@dataclass
class AsmParams:
    magic: str = "ALPN"
    l1_view: GenesisL1View = field(default_factory=GenesisL1View)
    admin: AdminSubprotocolConfig = field(
        default_factory=lambda: AdminSubprotocolConfig(
            strata_administrator=ThresholdConfig(keys=[compressed_pubkey(Key())]),
            strata_sequencer_manager=ThresholdConfig(keys=[compressed_pubkey(Key())]),
        )
    )
    checkpoint: CheckpointSubprotocolConfig = field(default_factory=CheckpointSubprotocolConfig)
    bridge: BridgeV1SubprotocolConfig = field(
        default_factory=lambda: BridgeV1SubprotocolConfig(
            operators=[even_pubkey(Key())],
        )
    )

    def as_json_string(self) -> str:
        d = {
            "magic": self.magic,
            "l1_view": asdict(self.l1_view),
            "subprotocols": [
                {"Admin": asdict(self.admin)},
                {"Checkpoint": asdict(self.checkpoint)},
                {"Bridge": asdict(self.bridge)},
            ],
        }
        return json.dumps(d, indent=2)

    def with_genesis_l1(self, genesis_l1: GenesisL1View):
        self.l1_view = genesis_l1
        self.checkpoint.genesis_l1_height = genesis_l1.blk.height
        return self


@dataclass
class DepositTxParams:
    magic_bytes: list[int] = field(default_factory=lambda: [0, 0, 0, 0])
    deposit_amount: int = field(default=100000)
    address: str = field(default="")
    operators_pubkey: str = field(default="")


@dataclass
class SyncParams:
    l1_follow_distance: int = field(default=6)
    client_checkpoint_interval: int = field(default=20)
    l2_blocks_fetch_limit: int = field(default=10)


@dataclass
class Params:
    rollup: RollupParams = field(default_factory=RollupParams)
    run: SyncParams = field(default_factory=SyncParams)
