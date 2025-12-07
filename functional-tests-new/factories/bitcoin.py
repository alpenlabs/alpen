"""
Bitcoin service factory.
Creates Bitcoin regtest nodes for testing.
"""

import os

import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from common.service import ServiceWrapper


class BitcoinFactory(flexitest.Factory):
    """
    Factory for creating Bitcoin regtest nodes.

    Usage:
        factory = BitcoinFactory(range(18443, 18543))
        bitcoin = factory.create_regtest()
        rpc = bitcoin.create_rpc()
    """

    def __init__(self, port_range: range):
        super().__init__(list(port_range))

    @flexitest.with_ectx("ctx")
    def create_regtest(
        self,
        rpc_user: str = "user",
        rpc_password: str = "password",
        **kwargs,
    ) -> ServiceWrapper:
        """
        Create a Bitcoin regtest node.

        Returns:
            Service with RPC access via .create_rpc()
        """
        ctx = kwargs["ctx"]  # The `with_ectx` ensures this is available. I don't like this though.
        datadir = ctx.make_service_dir("bitcoin")
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

        props = {
            "p2p_port": p2p_port,
            "rpc_port": rpc_port,
            "rpc_user": rpc_user,
            "rpc_url": rpc_url,
            "rpc_password": rpc_password,
            "walletname": "testwallet",
        }

        def make_rpc() -> BitcoindClient:
            return BitcoindClient(base_url=rpc_url, network="regtest")

        svc = ServiceWrapper(props, cmd, stdout=logfile, rpc_factory=make_rpc, name="bitcoin")
        svc.start()

        return svc
