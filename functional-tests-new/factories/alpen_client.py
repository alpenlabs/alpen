"""
Alpen-client node factory.
Creates alpen-client sequencer and fullnode instances.
"""

import contextlib
import os
import secrets
from pathlib import Path

import flexitest

from common.services import AlpenClientProps, AlpenClientService


def generate_p2p_secret_key() -> str:
    """Generate a random 32-byte hex-encoded P2P secret key."""
    return secrets.token_hex(32)


def generate_sequencer_keypair() -> tuple[str, str]:
    """
    Generate a sequencer keypair (private key, X-only public key).

    Returns:
        Tuple of (private_key_hex, public_key_hex) - both 32 bytes hex-encoded

    Note:
        The public key is the X-only public key (32 bytes) derived from the
        private key using secp256k1. This is required for Schnorr signature
        verification in the gossip protocol.
    """
    # Use a deterministic test keypair with a properly derived public key
    # Private key: 0x0101...01 (32 bytes of 0x01)
    # Public key: derived X-only public key from the private key
    privkey = "0x" + "01" * 32
    # This X-only public key was derived from the private key using secp256k1
    pubkey = "0x1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f"
    return privkey, pubkey


class AlpenClientFactory(flexitest.Factory):
    """
    Factory for creating alpen-client nodes.
    """

    def __init__(self, port_range: range):
        ports = list(port_range)
        if any(p < 1024 or p > 65535 for p in ports):
            raise ValueError(
                f"Port range must be between 1024 and 65535. "
                f"Got: {port_range.start}-{port_range.stop - 1}"
            )
        super().__init__(ports)

    @flexitest.with_ectx("ctx")
    def create_sequencer(
        self,
        sequencer_pubkey: str,
        sequencer_privkey: str,
        p2p_secret_key: str | None = None,
        enable_discovery: bool = False,
        custom_chain: str = "dev",
        **kwargs,
    ) -> AlpenClientService:
        """
        Create an alpen-client sequencer node.

        Args:
            sequencer_pubkey: Sequencer's public key (hex, 32 bytes)
            sequencer_privkey: Sequencer's private key (hex, 32 bytes) - set as env var
            p2p_secret_key: P2P secret key for deterministic enode (hex, 32 bytes)
            enable_discovery: Enable discv5 peer discovery (for bootnode mode)
            custom_chain: Chain spec to use
        """
        ctx: flexitest.EnvContext = kwargs["ctx"]

        datadir = Path(ctx.make_service_dir("alpen_client_sequencer"))
        http_port = self.next_port()
        p2p_port = self.next_port()
        authrpc_port = self.next_port()
        logfile = datadir / "service.log"

        # Generate P2P secret key if not provided
        if p2p_secret_key is None:
            p2p_secret_key = generate_p2p_secret_key()

        # Write P2P secret key to file (alpen-client expects hex string in file)
        p2p_secret_key_file = datadir / "p2p_secret_key"
        # Remove 0x prefix if present
        key_hex = p2p_secret_key.removeprefix("0x")
        p2p_secret_key_file.write_text(key_hex)

        # fmt: off
        cmd = [
            "alpen-client",
            "--datadir", str(datadir),
            "--sequencer",
            "--sequencer-pubkey", sequencer_pubkey,
            "--dummy-ol-client",  # Use DummyOLClient (no real OL connection needed)
            "--addr", "127.0.0.1",  # Force IPv4 for testing
            "--nat", "extip:127.0.0.1",  # Force enode to show 127.0.0.1
            "--port", str(p2p_port),
            "--http",
            "--http.port", str(http_port),
            "--http.api", "eth,net,admin,debug",
            "--authrpc.port", str(authrpc_port),
            "--p2p-secret-key", str(p2p_secret_key_file),
            "--custom-chain", custom_chain,
            "-vvvv",
        ]
        # fmt: on

        # Discovery mode configuration:
        # - enable_discovery=True: Use discv5 only (disable discv4)
        # - enable_discovery=False: Disable all discovery (rely on admin_addPeer/trusted-peers)
        if enable_discovery:
            discv5_port = self.next_port()
            # fmt: off
            cmd.extend([
                "--disable-discv4-discovery",  # Don't use legacy discv4
                "--enable-discv5-discovery",
                "--discovery.v5.addr", "127.0.0.1",
                "--discovery.v5.port", str(discv5_port),
            ])
            # fmt: on
        else:
            # Disable all discovery - peers connect via admin_addPeer or --trusted-peers
            cmd.append("-d")

        http_url = f"http://127.0.0.1:{http_port}"

        props: AlpenClientProps = {
            "http_port": http_port,
            "http_url": http_url,
            "p2p_port": p2p_port,
            "datadir": str(datadir),
            "mode": "sequencer",
            "enode": None,  # Will be populated after start
        }

        # Set environment variable for sequencer private key
        env = os.environ.copy()
        env["SEQUENCER_PRIVATE_KEY"] = sequencer_privkey

        svc = AlpenClientService(
            props,
            cmd,
            stdout=str(logfile),
            name="alpen_client_sequencer",
            env=env,
        )
        svc.stop_timeout = 30

        try:
            svc.start()
        except Exception as e:
            with contextlib.suppress(Exception):
                svc.stop()
            raise RuntimeError(f"Failed to start alpen-client sequencer: {e}") from e

        return svc

    @flexitest.with_ectx("ctx")
    def create_fullnode(
        self,
        sequencer_pubkey: str,
        trusted_peers: list[str] | None = None,
        bootnodes: list[str] | None = None,
        enable_discovery: bool = False,
        p2p_secret_key: str | None = None,
        custom_chain: str = "dev",
        instance_id: int = 0,
        datadir_override: str | None = None,
        **kwargs,
    ) -> AlpenClientService:
        """
        Create an alpen-client fullnode.

        Args:
            sequencer_pubkey: Sequencer's public key for signature validation
            trusted_peers: List of enode URLs to connect to (direct connection)
            bootnodes: List of enode URLs for discovery bootstrap
            enable_discovery: Enable discv5 peer discovery
            p2p_secret_key: P2P secret key for deterministic enode
            custom_chain: Chain spec to use
            instance_id: Instance ID for multiple fullnodes
            datadir_override: Optional datadir path (bypasses EnvContext requirement)
        """
        if datadir_override:
            datadir = Path(datadir_override)
            datadir.mkdir(parents=True, exist_ok=True)
        else:
            ctx: flexitest.EnvContext = kwargs["ctx"]
            datadir = Path(ctx.make_service_dir(f"alpen_client_fullnode_{instance_id}"))
        http_port = self.next_port()
        p2p_port = self.next_port()
        authrpc_port = self.next_port()
        logfile = datadir / "service.log"

        # Generate P2P secret key if not provided
        if p2p_secret_key is None:
            p2p_secret_key = generate_p2p_secret_key()

        # Write P2P secret key to file (alpen-client expects hex string in file)
        p2p_secret_key_file = datadir / "p2p_secret_key"
        # Remove 0x prefix if present
        key_hex = p2p_secret_key.removeprefix("0x")
        p2p_secret_key_file.write_text(key_hex)

        # fmt: off
        cmd = [
            "alpen-client",
            "--datadir", str(datadir),
            "--sequencer-pubkey", sequencer_pubkey,
            "--dummy-ol-client",  # Use DummyOLClient (no real OL connection needed)
            "--addr", "127.0.0.1",  # Force IPv4 for testing
            "--nat", "extip:127.0.0.1",  # Force enode to show 127.0.0.1
            "--port", str(p2p_port),
            "--http",
            "--http.port", str(http_port),
            "--http.api", "eth,net,admin,debug",
            "--authrpc.port", str(authrpc_port),
            "--p2p-secret-key", str(p2p_secret_key_file),
            "--custom-chain", custom_chain,
            "-vvvv",
        ]
        # fmt: on

        # Add trusted peers if provided
        if trusted_peers:
            cmd.extend(["--trusted-peers", ",".join(trusted_peers)])

        # Add bootnodes if provided (requires discovery to be enabled)
        if bootnodes:
            cmd.extend(["--bootnodes", ",".join(bootnodes)])

        # Discovery mode configuration:
        # - enable_discovery=True: Use discv5 only (disable discv4)
        # - enable_discovery=False: Disable all discovery (rely on admin_addPeer/trusted-peers)
        if enable_discovery:
            discv5_port = self.next_port()
            # fmt: off
            cmd.extend([
                "--disable-discv4-discovery",  # Don't use legacy discv4
                "--enable-discv5-discovery",
                "--discovery.v5.addr", "127.0.0.1",
                "--discovery.v5.port", str(discv5_port),
            ])
            # fmt: on
        else:
            # Disable all discovery - peers connect via admin_addPeer or --trusted-peers
            cmd.append("-d")

        http_url = f"http://127.0.0.1:{http_port}"

        props: AlpenClientProps = {
            "http_port": http_port,
            "http_url": http_url,
            "p2p_port": p2p_port,
            "datadir": str(datadir),
            "mode": "fullnode",
            "enode": None,
        }

        svc = AlpenClientService(
            props,
            cmd,
            stdout=str(logfile),
            name=f"alpen_client_fullnode_{instance_id}",
        )
        svc.stop_timeout = 30

        try:
            svc.start()
        except Exception as e:
            with contextlib.suppress(Exception):
                svc.stop()
            raise RuntimeError(f"Failed to start alpen-client fullnode: {e}") from e

        return svc
