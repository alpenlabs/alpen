"""
Alpen-client test environment configurations.
"""

from typing import cast

import flexitest

from common.config import ServiceType
from factories.alpen_client import AlpenClientFactory, generate_sequencer_keypair


class AlpenClientEnv(flexitest.EnvConfig):
    """
    Configurable alpen-client environment: 1 sequencer + N fullnodes.

    Parameters:
        fullnode_count: Number of fullnodes (default 1)
        enable_discovery: Enable discv5 discovery (default False)
        pure_discovery: If True, rely only on bootnode discovery (no admin_addPeer).
                        Requires enable_discovery=True. (default False)
        mesh_bootnodes: If True, each fullnode uses previous fullnodes as bootnodes
                        (in addition to sequencer) to help form mesh topology.
                        Requires enable_discovery=True. (default False)
    """

    def __init__(
        self,
        fullnode_count: int = 1,
        enable_discovery: bool = False,
        pure_discovery: bool = False,
        mesh_bootnodes: bool = False,
    ):
        self.fullnode_count = fullnode_count
        self.enable_discovery = enable_discovery
        self.pure_discovery = pure_discovery
        self.mesh_bootnodes = mesh_bootnodes
        if pure_discovery and not enable_discovery:
            raise ValueError("pure_discovery requires enable_discovery=True")
        if mesh_bootnodes and not enable_discovery:
            raise ValueError("mesh_bootnodes requires enable_discovery=True")

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        factory = cast(AlpenClientFactory, ectx.get_factory(ServiceType.AlpenClient))
        privkey, pubkey = generate_sequencer_keypair()

        # Start sequencer
        sequencer = factory.create_sequencer(
            sequencer_pubkey=pubkey,
            sequencer_privkey=privkey,
            enable_discovery=self.enable_discovery,
        )
        sequencer.wait_for_ready(timeout=60)
        seq_enode = sequencer.get_enode()

        services = {"sequencer": sequencer}
        fullnodes = []
        fn_enodes = []  # Track fullnode enodes for mesh bootnodes

        # Start fullnodes
        for i in range(self.fullnode_count):
            # Build bootnode list
            bootnodes = None
            if self.enable_discovery:
                bootnodes = [seq_enode]
                # Add previous fullnodes as bootnodes for mesh formation
                if self.mesh_bootnodes:
                    bootnodes.extend(fn_enodes)

            fullnode = factory.create_fullnode(
                sequencer_pubkey=pubkey,
                bootnodes=bootnodes,
                enable_discovery=self.enable_discovery,
                instance_id=i,
            )
            fullnode.wait_for_ready(timeout=60)
            fullnodes.append(fullnode)

            # Track enode for mesh bootnodes
            if self.mesh_bootnodes:
                fn_enodes.append(fullnode.get_enode())

            # Use "fullnode" for single, "fullnode_N" for multiple
            key = "fullnode" if self.fullnode_count == 1 else f"fullnode_{i}"
            services[key] = fullnode

        # Connect fullnodes to sequencer via admin_addPeer (unless pure_discovery mode)
        if not self.pure_discovery:
            seq_rpc = sequencer.create_rpc()
            for fn in fullnodes:
                fn_enode = fn.get_enode()
                seq_rpc.admin_addPeer(fn_enode)

        return flexitest.LiveEnv(services)
