"""
Alpen-client test environment configurations.
"""

import flexitest
from pathlib import Path

from common.config.config import EpochSealingConfig
from common.config.constants import ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from envconfigs.alpen_client import AlpenClientEnv
from envconfigs.strata import StrataEnvConfig


class EeOLEnv(flexitest.EnvConfig):
    """
    Configurable EE-OL env.

    Parameters:
        fullnode_count: Number of fullnodes (default 1)
        enable_discovery: Enable discv5 discovery (default False)
        pure_discovery: If True, rely only on bootnode discovery (no admin_addPeer).
                        Requires enable_discovery=True. (default False)
        mesh_bootnodes: If True, each fullnode uses previous fullnodes as bootnodes
                        (in addition to sequencer) to help form mesh topology.
                        Requires enable_discovery=True. (default False)
        pre_generate_blocks: How many bitcoin blocks to pre-generate
        batch_sealing_block_count: Number of EE blocks before sealing an alpen-client DA batch
    """

    def __init__(
        self,
        fullnode_count: int = 1,
        enable_discovery: bool = False,
        pure_discovery: bool = False,
        mesh_bootnodes: bool = False,
        pre_generate_blocks: int = 0,
        seal_epoch_slots: int | None = None,
        admin_confirmation_depth: int | None = None,
        fund_test_cli_wallet: bool = False,
        ol_block_time_ms: int | None = None,
        dev_track_finalized_epoch: bool = False,
        batch_sealing_block_count: int = 10,
        bridge_operator_pubkeys: list[str] | None = None,
        custom_chain: str = "dev",
        alpen_chain_config: Path | None = None,
    ):
        self.fullnode_count = fullnode_count
        self.enable_discovery = enable_discovery
        self.pure_discovery = pure_discovery
        self.mesh_bootnodes = mesh_bootnodes
        self.pre_generate_blocks = pre_generate_blocks
        self.admin_confirmation_depth = admin_confirmation_depth
        self.fund_test_cli_wallet = fund_test_cli_wallet
        self.ol_block_time_ms = ol_block_time_ms
        self.dev_track_finalized_epoch = dev_track_finalized_epoch
        self.batch_sealing_block_count = batch_sealing_block_count
        self.bridge_operator_pubkeys = bridge_operator_pubkeys
        self.custom_chain = custom_chain
        self.alpen_chain_config = alpen_chain_config
        self.epoch_seal_config = (
            EpochSealingConfig.new_fixed_slot(seal_epoch_slots)
            if seal_epoch_slots
            else EpochSealingConfig()
        )
        if pure_discovery and not enable_discovery:
            raise ValueError("pure_discovery requires enable_discovery=True")
        if mesh_bootnodes and not enable_discovery:
            raise ValueError("mesh_bootnodes requires enable_discovery=True")

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        strata_config = StrataEnvConfig(
            pre_generate_blocks=self.pre_generate_blocks,
            epoch_sealing=self.epoch_seal_config,
            fund_test_cli_wallet=self.fund_test_cli_wallet,
            admin_confirmation_depth=self.admin_confirmation_depth,
            ol_block_time_ms=self.ol_block_time_ms,
            bridge_operator_pubkeys=self.bridge_operator_pubkeys,
            alpen_chain_config=self.alpen_chain_config,
        )
        strata_services = strata_config._get_services(ectx)

        # Get and pass ol endpoint
        seq: StrataService = strata_services[ServiceType.Strata]
        bitcoin: BitcoinService = strata_services[ServiceType.Bitcoin]
        ol_endpoint = seq.props["rpc_url"]

        alpen_services = AlpenClientEnv.get_services(
            ectx,
            self.enable_discovery,
            self.fullnode_count,
            self.mesh_bootnodes,
            self.pure_discovery,
            bitcoin_service=bitcoin,
            ol_endpoint=ol_endpoint,
            dev_track_finalized_epoch=self.dev_track_finalized_epoch,
            batch_sealing_block_count=self.batch_sealing_block_count,
            custom_chain=self.custom_chain,
        )

        services = {**alpen_services, **strata_services}
        return flexitest.LiveEnv(services)
