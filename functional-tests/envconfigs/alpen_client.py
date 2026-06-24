"""
Alpen-client test environment configurations.
"""

from dataclasses import dataclass
from pathlib import Path
from typing import cast

import flexitest

from common.config import EeDaConfig, FeeModelConfig, ServiceType
from common.services.bitcoin import BitcoinService
from factories.alpen_client import AlpenClientFactory, generate_sequencer_keypair
from factories.bitcoin import BitcoinFactory

# Default magic bytes for DA testing (must be 4 bytes)
DEFAULT_DA_MAGIC_BYTES = b"ALPN"
DA_WALLET_FUNDING_OUTPUTS = 25
INITIAL_L1_MATURITY_BLOCKS = 101
INITIAL_L1_MINE_CHUNK_SIZE = 10


def _generate_blocks_in_chunks(btc_rpc, block_count: int, mining_address: str) -> None:
    """Mine regtest blocks without relying on one long-running RPC call."""
    remaining = block_count
    while remaining > 0:
        chunk = min(remaining, INITIAL_L1_MINE_CHUNK_SIZE)
        btc_rpc.proxy.generatetoaddress(chunk, mining_address)
        remaining -= chunk


@dataclass
class AlpenClientEnvParams:
    fullnode_count: int
    enable_discovery: bool
    pure_discovery: bool
    mesh_bootnodes: bool
    enable_l1_da: bool = False
    da_magic_bytes: bytes = b"ALPN"
    l1_reorg_safe_depth: int = 2
    batch_sealing_block_count: int = 10
    dev_track_latest_epoch: bool = False
    beneficiary_address: str | None = None
    fee_model: FeeModelConfig | None = None


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
        da_magic_bytes: 4-byte magic for OP_RETURN tagging (default: b"ALPN")
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
        beneficiary_address: str | None = None,
        fee_model: FeeModelConfig | None = None,
    ):
        self.env_params = AlpenClientEnvParams(
            fullnode_count=fullnode_count,
            enable_discovery=enable_discovery,
            pure_discovery=pure_discovery,
            mesh_bootnodes=mesh_bootnodes,
            enable_l1_da=enable_l1_da,
            da_magic_bytes=da_magic_bytes,
            l1_reorg_safe_depth=l1_reorg_safe_depth,
            batch_sealing_block_count=batch_sealing_block_count,
            beneficiary_address=beneficiary_address,
            fee_model=fee_model,
        )
        if pure_discovery and not enable_discovery:
            raise ValueError("pure_discovery requires enable_discovery=True")
        if mesh_bootnodes and not enable_discovery:
            raise ValueError("mesh_bootnodes requires enable_discovery=True")
        if len(da_magic_bytes) != 4:
            raise ValueError(f"da_magic_bytes must be exactly 4 bytes, got {len(da_magic_bytes)}")

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        services = self.get_services(ectx, self.env_params)
        return flexitest.LiveEnv(services)

    @staticmethod
    def get_services(
        ectx: flexitest.EnvContext,
        envparams: AlpenClientEnvParams,
        bitcoin_service: BitcoinService | None = None,
        ol_endpoint: str | None = None,
        ol_submit_endpoint: str | None = None,
        ol_submit_token: str | None = None,
        ee_params_path: Path | None = None,
    ):
        factory = cast(AlpenClientFactory, ectx.get_factory(ServiceType.AlpenClient))
        privkey, pubkey = generate_sequencer_keypair()

        services = {}
        da_config = None

        # Start Bitcoin if DA is enabled
        if envparams.enable_l1_da:
            if bitcoin_service is None:
                btc_factory = cast(BitcoinFactory, ectx.get_factory(ServiceType.Bitcoin))
                bitcoin = btc_factory.create_regtest()
                bitcoin.wait_for_ready(timeout=30)

                btc_rpc = bitcoin.create_rpc()
                btc_rpc.proxy.createwallet("testwallet")
                mining_address = btc_rpc.proxy.getnewaddress()
                _generate_blocks_in_chunks(btc_rpc, INITIAL_L1_MATURITY_BLOCKS, mining_address)

                # DA publishing can sign multiple commits before earlier
                # change outputs become confirmed spendable. Split a matured
                # coinbase into normal wallet UTXOs so the signer has an
                # independent spend source for each pending commit.
                funding_outputs = {
                    btc_rpc.proxy.getnewaddress(): 1 for _ in range(DA_WALLET_FUNDING_OUTPUTS)
                }
                btc_rpc.proxy.sendmany("", funding_outputs)
                btc_rpc.proxy.generatetoaddress(1, mining_address)
            else:
                bitcoin = bitcoin_service

            btc_rpc = bitcoin.create_rpc()

            genesis_l1_height = btc_rpc.proxy.getblockcount()

            # Construct clean RPC URL without credentials (Rust BtcClient expects separate auth)
            btc_rpc_url = f"http://localhost:{bitcoin.props['rpc_port']}"

            da_config = EeDaConfig(
                btc_rpc_url=btc_rpc_url,
                btc_rpc_user=bitcoin.props["rpc_user"],
                btc_rpc_password=bitcoin.props["rpc_password"],
                magic_bytes=envparams.da_magic_bytes,
                l1_reorg_safe_depth=envparams.l1_reorg_safe_depth,
                genesis_l1_height=genesis_l1_height,
                batch_sealing_block_count=envparams.batch_sealing_block_count,
            )
            services[ServiceType.Bitcoin] = bitcoin

        # Start sequencer
        sequencer = factory.create_sequencer(
            sequencer_pubkey=pubkey,
            sequencer_privkey=privkey,
            enable_discovery=envparams.enable_discovery,
            ol_endpoint=ol_endpoint,
            ol_submit_endpoint=ol_submit_endpoint,
            ol_submit_token=ol_submit_token,
            ee_params_path=ee_params_path,
            da_config=da_config,
            batch_sealing_block_count=envparams.batch_sealing_block_count,
            dev_track_latest_epoch=envparams.dev_track_latest_epoch,
            beneficiary_address=envparams.beneficiary_address,
            fee_model=envparams.fee_model,
        )
        sequencer.wait_for_ready(timeout=60)
        seq_enode = sequencer.get_enode()
        seq_http_url = sequencer.props["http_url"]

        services[ServiceType.AlpenSequencer] = sequencer
        fullnodes = []
        fn_enodes = []  # Track fullnode enodes for mesh bootnodes

        # Start fullnodes
        for i in range(envparams.fullnode_count):
            # Build bootnode list
            bootnodes = None
            if envparams.enable_discovery:
                bootnodes = [seq_enode]
                # Add previous fullnodes as bootnodes for mesh formation
                if envparams.mesh_bootnodes:
                    bootnodes.extend(fn_enodes)

            fullnode = factory.create_fullnode(
                sequencer_pubkey=pubkey,
                bootnodes=bootnodes,
                enable_discovery=envparams.enable_discovery,
                instance_id=i,
                sequencer_http=seq_http_url,  # Forward transactions to sequencer
                ol_endpoint=ol_endpoint,
                ee_params_path=ee_params_path,
            )
            fullnode.wait_for_ready(timeout=60)
            fullnodes.append(fullnode)

            # Track enode for mesh bootnodes
            if envparams.mesh_bootnodes:
                fn_enodes.append(fullnode.get_enode())

            # Use "fullnode" for single, "fullnode_N" for multiple
            key = (
                ServiceType.AlpenFullNode
                if envparams.fullnode_count == 1
                else f"{ServiceType.AlpenFullNode}_{i}"
            )
            services[key] = fullnode

        # Connect fullnodes to sequencer via admin_addPeer (unless pure_discovery mode)
        if not envparams.pure_discovery:
            seq_rpc = sequencer.create_rpc()
            for fn in fullnodes:
                fn_enode = fn.get_enode()
                seq_rpc.admin_addPeer(fn_enode)
        return services
