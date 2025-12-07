"""
Rollup consensus parameters dataclasses.
"""

import json
from dataclasses import dataclass, field, asdict
from typing import Literal, Tuple

from bitcoinlib.keys import Key


def hex_bytes_repeated(n: int, repeat=32):
    n = n if n < 256 and n >= 0 else 0
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


# TODO: move this to some place common as this should be useful for other purposes as well
def gen_random_keypair() -> Tuple[str, Key]:
    """Generates a keypair and returns a tuple of xonly pubkey and privkey."""
    key = Key()
    xpubkey = format(key.x, "064x")
    return xpubkey, key


@dataclass
class OperatorPubkeys:
    wallet_pk: str = field(default_factory=lambda: gen_random_keypair()[0])
    signing_pk: str = field(default_factory=lambda: gen_random_keypair()[0])


@dataclass
class OperatorConfig:
    static: list[OperatorPubkeys] = field(default_factory=lambda: [OperatorPubkeys()])


@dataclass
class ProofPublishModeTimeout:
    timeout: int = field(default=30)


ProofPublishMode = Literal["strict"] | ProofPublishModeTimeout


@dataclass
class Checked:
    schnorr_key: str  # TODO: more sophisticated, using pydantic?


CredRule = Checked | Literal["unchecked"]


@dataclass
class RollupParams:
    magic_bytes: list[int] = field(default_factory=lambda: [0, 0, 0, 0])
    block_time: int = field(default=5000)  # millisecs
    da_tag: str = field(default="strata-da")
    checkpoint_tag: str = field(default="strata-ckpt")
    cred_rule: CredRule = field(default="unchecked")
    genesis_l1_view: GenesisL1View = field(default_factory=GenesisL1View)
    operator_config: OperatorConfig = field(default_factory=OperatorConfig)
    evm_genesis_block_hash: str = field(default_factory=lambda: hex_bytes_repeated(0))
    evm_genesis_block_state_root: str = field(default_factory=lambda: hex_bytes_repeated(0))
    l1_reorg_safe_depth: int = field(default=6)
    target_l2_batch_size: int = field(default=64)
    max_address_length: int = field(default=32)
    deposit_amount: int = field(default=100000)
    checkpoint_predicate: str = field(default="AlwaysAccept")
    dispatch_assignment_dur: int = field(default=144)
    proof_publish_mode: ProofPublishMode = field(default_factory=ProofPublishModeTimeout)
    max_deposits_in_block: int = field(default=10)
    network: str = field(default="regtest")

    def as_json_string(self) -> str:
        d = asdict(self)
        return json.dumps(d, indent=2)


@dataclass
class DepositTxParams:
    magic_bytes: list[int] = field(default_factory=lambda: [0, 0, 0, 0])
    max_address_length: int = field(default=128)
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
