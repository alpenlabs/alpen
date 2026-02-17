"""
Alpen-client test environment configurations.
"""

from typing import cast

import flexitest

from common.config import EeDaConfig, ServiceType
from factories.alpen_client import AlpenClientFactory, generate_sequencer_keypair
from factories.bitcoin import BitcoinFactory

# Default magic bytes for DA testing (must be 4 bytes)
DEFAULT_DA_MAGIC_BYTES = b"ALPT"


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
        enable_l1_da: Enable DA pipeline for posting state diffs to Bitcoin L1 (default False)
        da_magic_bytes: 4-byte magic for OP_RETURN tagging (default: b"ALPT")
        l1_reorg_safe_depth: Confirmation depth for L1 transactions (default: 1)
        batch_sealing_block_count: Number of blocks before sealing a batch (default: 5)
    """

    def __init__(
        self,
        fullnode_count: int = 1,
        enable_discovery: bool = False,
        pure_discovery: bool = False,
        mesh_bootnodes: bool = False,
        enable_l1_da: bool = False,
        da_magic_bytes: bytes = DEFAULT_DA_MAGIC_BYTES,
        l1_reorg_safe_depth: int = 1,
        batch_sealing_block_count: int = 5,
    ):
        self.fullnode_count = fullnode_count
        self.enable_discovery = enable_discovery
        self.pure_discovery = pure_discovery
        self.mesh_bootnodes = mesh_bootnodes
        self.enable_l1_da = enable_l1_da
        self.da_magic_bytes = da_magic_bytes
        self.l1_reorg_safe_depth = l1_reorg_safe_depth
        self.batch_sealing_block_count = batch_sealing_block_count
        if pure_discovery and not enable_discovery:
            raise ValueError("pure_discovery requires enable_discovery=True")
        if mesh_bootnodes and not enable_discovery:
            raise ValueError("mesh_bootnodes requires enable_discovery=True")
        if len(da_magic_bytes) != 4:
            raise ValueError(f"da_magic_bytes must be exactly 4 bytes, got {len(da_magic_bytes)}")

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        services = self.get_services(
            ectx,
            self.enable_discovery,
            self.fullnode_count,
            self.mesh_bootnodes,
            self.pure_discovery,
            self.enable_l1_da,
            self.da_magic_bytes,
            self.l1_reorg_safe_depth,
            self.batch_sealing_block_count,
        )
        return flexitest.LiveEnv(services)

    @staticmethod
    def get_services(
        ectx: flexitest.EnvContext,
        enable_discovery: bool,
        fullnode_count: int,
        mesh_bootnodes: int,
        pure_discovery: bool,
        enable_l1_da: bool = False,
        da_magic_bytes: bytes = b'',
        l1_reorg_safe_depth: int = 2,
        batch_sealing_block_count: int = 10,
        ol_endpoint: str | None = None,
    ):
        factory = cast(AlpenClientFactory, ectx.get_factory(ServiceType.AlpenClient))
        privkey, pubkey = generate_sequencer_keypair()

        services = {}
        da_config = None

        # Start Bitcoin if DA is enabled
        if enable_l1_da:
            btc_factory = cast(BitcoinFactory, ectx.get_factory(ServiceType.Bitcoin))
            bitcoin = btc_factory.create_regtest()
            bitcoin.wait_for_ready(timeout=30)

            btc_rpc = bitcoin.create_rpc()
            btc_rpc.proxy.createwallet("testwallet")
            address = btc_rpc.proxy.getnewaddress()
            btc_rpc.proxy.generatetoaddress(101, address)

            genesis_l1_height = btc_rpc.proxy.getblockcount()

            # Construct clean RPC URL without credentials (Rust BtcClient expects separate auth)
            btc_rpc_url = f"http://localhost:{bitcoin.props['rpc_port']}"

            da_config = EeDaConfig(
                btc_rpc_url=btc_rpc_url,
                btc_rpc_user=bitcoin.props["rpc_user"],
                btc_rpc_password=bitcoin.props["rpc_password"],
                magic_bytes=da_magic_bytes,
                l1_reorg_safe_depth=l1_reorg_safe_depth,
                genesis_l1_height=genesis_l1_height,
                batch_sealing_block_count=batch_sealing_block_count,
            )
            services["bitcoin"] = bitcoin

        # Start sequencer
        sequencer = factory.create_sequencer(
            sequencer_pubkey=pubkey,
            sequencer_privkey=privkey,
            enable_discovery=enable_discovery,
            ol_endpoint=ol_endpoint,
            da_config=da_config,
        )
        sequencer.wait_for_ready(timeout=60)
        seq_enode = sequencer.get_enode()
        seq_http_url = sequencer.props["http_url"]

        services["alpen_sequencer"] = sequencer
        fullnodes = []
        fn_enodes = []  # Track fullnode enodes for mesh bootnodes

        # Start fullnodes
        for i in range(fullnode_count):
            # Build bootnode list
            bootnodes = None
            if enable_discovery:
                bootnodes = [seq_enode]
                # Add previous fullnodes as bootnodes for mesh formation
                if mesh_bootnodes:
                    bootnodes.extend(fn_enodes)

            fullnode = factory.create_fullnode(
                sequencer_pubkey=pubkey,
                bootnodes=bootnodes,
                enable_discovery=enable_discovery,
                instance_id=i,
                sequencer_http=seq_http_url,  # Forward transactions to sequencer
                ol_endpoint=ol_endpoint,
            )
            fullnode.wait_for_ready(timeout=60)
            fullnodes.append(fullnode)

            # Track enode for mesh bootnodes
            if mesh_bootnodes:
                fn_enodes.append(fullnode.get_enode())

            # Use "fullnode" for single, "fullnode_N" for multiple
            key = "alpen_fullnode" if fullnode_count == 1 else f"alpen_fullnode_{i}"
            services[key] = fullnode

        # Connect fullnodes to sequencer via admin_addPeer (unless pure_discovery mode)
        if not pure_discovery:
            seq_rpc = sequencer.create_rpc()
            for fn in fullnodes:
                fn_enode = fn.get_enode()
                seq_rpc.admin_addPeer(fn_enode)
        return services
