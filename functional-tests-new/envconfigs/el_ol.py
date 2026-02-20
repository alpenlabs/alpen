"""
Alpen-client test environment configurations.
"""

import flexitest

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
    """

    def __init__(
        self,
        fullnode_count: int = 1,
        enable_discovery: bool = False,
        pure_discovery: bool = False,
        mesh_bootnodes: bool = False,
        pre_generate_blocks: int = 0,
    ):
        self.fullnode_count = fullnode_count
        self.enable_discovery = enable_discovery
        self.pure_discovery = pure_discovery
        self.mesh_bootnodes = mesh_bootnodes
        self.pre_generate_blocks = pre_generate_blocks
        if pure_discovery and not enable_discovery:
            raise ValueError("pure_discovery requires enable_discovery=True")
        if mesh_bootnodes and not enable_discovery:
            raise ValueError("mesh_bootnodes requires enable_discovery=True")

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        strata_services = StrataEnvConfig.get_services(ectx, self.pre_generate_blocks)

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
        )

        services = {**alpen_services, **strata_services}
        return flexitest.LiveEnv(services)
