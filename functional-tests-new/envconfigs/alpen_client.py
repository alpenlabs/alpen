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
    """

    def __init__(
        self,
        fullnode_count: int = 1,
        enable_discovery: bool = False,
        pure_discovery: bool = False,
    ):
        self.fullnode_count = fullnode_count
        self.enable_discovery = enable_discovery
        self.pure_discovery = pure_discovery
        if pure_discovery and not enable_discovery:
            raise ValueError("pure_discovery requires enable_discovery=True")

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

        # Start fullnodes
        for i in range(self.fullnode_count):
            fullnode = factory.create_fullnode(
                sequencer_pubkey=pubkey,
                bootnodes=[seq_enode] if self.enable_discovery else None,
                enable_discovery=self.enable_discovery,
                instance_id=i,
            )
            fullnode.wait_for_ready(timeout=60)
            fullnodes.append(fullnode)

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


class AlpenClientRelayEnv(flexitest.EnvConfig):
    """
    Fullnode relay topology: fullnode_0 is the hub.

    Topology:
        fullnode_0 (started first, acts as hub)
            ├── sequencer
            └── fullnode_1

    Blocks relay: sequencer → fullnode_0 → fullnode_1
    """

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        factory = cast(AlpenClientFactory, ectx.get_factory(ServiceType.AlpenClient))
        privkey, pubkey = generate_sequencer_keypair()

        # Start fullnode_0 FIRST as the hub
        fullnode_0 = factory.create_fullnode(
            sequencer_pubkey=pubkey,
            enable_discovery=True,
            instance_id=0,
        )
        fullnode_0.wait_for_ready(timeout=60)
        fn0_enode = fullnode_0.get_enode()

        # Start sequencer
        sequencer = factory.create_sequencer(
            sequencer_pubkey=pubkey,
            sequencer_privkey=privkey,
            enable_discovery=True,
        )
        sequencer.wait_for_ready(timeout=60)

        # Start fullnode_1
        fullnode_1 = factory.create_fullnode(
            sequencer_pubkey=pubkey,
            enable_discovery=False,
            instance_id=1,
        )
        fullnode_1.wait_for_ready(timeout=60)
        fn1_enode = fullnode_1.get_enode()

        # Connect via admin_addPeer:
        # - sequencer ↔ fullnode_0
        # - fullnode_0 ↔ fullnode_1
        fn0_rpc = fullnode_0.create_rpc()
        seq_rpc = sequencer.create_rpc()

        seq_rpc.admin_addPeer(fn0_enode)
        fn0_rpc.admin_addPeer(fn1_enode)

        services = {
            "sequencer": sequencer,
            "fullnode_0": fullnode_0,
            "fullnode_1": fullnode_1,
        }

        return flexitest.LiveEnv(services)
