"""
Configuration dataclasses for services.
"""

from dataclasses import asdict, dataclass, field

import toml


@dataclass
class ClientConfig:
    rpc_host: str = field(default="")
    rpc_port: int = field(default=0)
    p2p_port: int = field(default=0)
    sync_endpoint: str | None = field(default=None)
    l2_blocks_fetch_limit: int = field(default=10)
    datadir: str = field(default="datadir")
    db_retry_count: int = field(default=3)


@dataclass
class SyncConfig:
    l1_follow_distance: int = field(default=6)
    client_checkpoint_interval: int = field(default=20)


@dataclass
class BitcoindConfig:
    rpc_url: str = field(default="http://localhost:8443")
    rpc_user: str = field(default="rpcuser")
    rpc_password: str = field(default="rpcpassword")
    network: str = field(default="regtest")
    retry_count: int | None = field(default=3)
    retry_interval: int | None = field(default=None)


@dataclass
class ReaderConfig:
    client_poll_dur_ms: int = field(default=200)


@dataclass
class WriterConfig:
    write_poll_dur_ms: int = field(default=200)
    reveal_amount: int = field(default=546)  # The dust amount
    fee_policy: str = field(default="smart")  # TODO: handle this as enum: Smart | Fixed(u64)
    bundle_interval_ms: int = field(default=200)


@dataclass
class BroadcasterConfig:
    poll_interval_ms: int = field(default=200)


@dataclass
class BtcioConfig:
    reader: ReaderConfig = field(default_factory=ReaderConfig)
    writer: WriterConfig = field(default_factory=WriterConfig)
    broadcaster: BroadcasterConfig = field(default_factory=BroadcasterConfig)


@dataclass
class RethELConfig:
    rpc_url: str = field(default="")
    secret: str = field(default="")


@dataclass
class ExecConfig:
    reth: RethELConfig = field(default_factory=RethELConfig)


@dataclass
class RelayerConfig:
    refresh_interval: int = field(default=200)
    stale_duration: int = field(default=20)
    relay_misc: bool = field(default=True)


@dataclass
class SequencerConfig:
    max_txs_per_block: int = field(default=100)


@dataclass
class EeDaConfig:
    """DA pipeline configuration for alpen-client sequencer.

    Configures the EE data availability pipeline that posts state diffs
    to Bitcoin L1 using chunked envelopes.
    """

    btc_rpc_url: str
    btc_rpc_user: str
    btc_rpc_password: str
    magic_bytes: bytes  # 4 bytes for OP_RETURN tagging
    l1_reorg_safe_depth: int = field(default=6)
    genesis_l1_height: int = field(default=0)
    batch_sealing_block_count: int = field(default=100)

    def __post_init__(self):
        if len(self.magic_bytes) != 4:
            raise ValueError(f"magic_bytes must be exactly 4 bytes, got {len(self.magic_bytes)}")

class EpochSealingConfig:
    policy: str = field(default="FixedSlot")
    slots_per_epoch: int | None = field(default=4)


@dataclass
class EpochSealingConfig:
    policy: str = field(default="FixedSlot")
    slots_per_epoch: int | None = field(default=4)


@dataclass
class StrataConfig:
    client: ClientConfig = field(default_factory=ClientConfig)
    bitcoind: BitcoindConfig = field(default_factory=BitcoindConfig)
    btcio: BtcioConfig = field(default_factory=BtcioConfig)
    sync: SyncConfig = field(default_factory=SyncConfig)
    exec: ExecConfig = field(default_factory=ExecConfig)
    relayer: RelayerConfig = field(default_factory=RelayerConfig)
    sequencer: SequencerConfig | None = field(default=None)
    epoch_sealing: EpochSealingConfig = field(default_factory=EpochSealingConfig)

    def as_toml_string(self) -> str:
        d = asdict(self)
        # Remove None values (optional configs)
        d = {k: v for k, v in d.items() if v is not None}
        return toml.dumps(d)
