"""
Bitcoin service factory.
Creates Bitcoin regtest nodes for testing.
"""

import contextlib
import os

import flexitest

from common.config import ServiceType
from common.services import BitcoinProps, BitcoinService


class BitcoinFactory(flexitest.Factory):
    """
    Factory for creating Bitcoin regtest nodes.

    Usage:
        factory = BitcoinFactory(range(18443, 18543))
        bitcoin = factory.create_regtest()
        rpc = bitcoin.create_rpc()
    """

    def __init__(self, port_range: range):
        ports = list(port_range)
        if any(p < 1024 or p > 65535 for p in ports):
            raise ValueError(
                f"BitcoinFactory: Port range must be between 1024 and 65535. "
                f"Got: {port_range.start}-{port_range.stop - 1}"
            )
        super().__init__(ports)

    @flexitest.with_ectx("ctx")
    def create_regtest(
        self,
        rpc_user: str = "user",
        rpc_password: str = "password",
        **kwargs,
    ) -> BitcoinService:
        """
        Create a Bitcoin regtest node.

        Returns:
            Service with RPC access via .create_rpc()
        """
        # The `with_ectx` ensures this is available. Don't like this though.
        ctx: flexitest.EnvContext = kwargs["ctx"]

        datadir = ctx.make_service_dir(ServiceType.Bitcoin)
        p2p_port = self.next_port()
        rpc_port = self.next_port()
        logfile = os.path.join(datadir, "service.log")

        cmd = [
            "bitcoind",
            "-txindex",
            "-regtest",
            "-listen=0",
            f"-port={p2p_port}",
            "-printtoconsole",
            "-fallbackfee=0.00001",
            "-minrelaytxfee=0",
            f"-datadir={datadir}",
            f"-rpcport={rpc_port}",
            f"-rpcuser={rpc_user}",
            f"-rpcpassword={rpc_password}",
        ]

        rpc_url = f"http://{rpc_user}:{rpc_password}@localhost:{rpc_port}"

        props: BitcoinProps = {
            "p2p_port": p2p_port,
            "rpc_port": rpc_port,
            "rpc_user": rpc_user,
            "rpc_url": rpc_url,
            "rpc_password": rpc_password,
            "datadir": datadir,
            "walletname": "testwallet",
        }

        svc = BitcoinService(props, cmd, stdout=logfile, name=ServiceType.Bitcoin)
        try:
            svc.start()
        except Exception as e:
            # Ensure cleanup on failure to prevent resource leaks
            with contextlib.suppress(Exception):
                svc.stop()
            raise RuntimeError(f"Failed to start bitcoin service: {e}") from e

        return svc
