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
        zmq_hashblock = self.next_port()
        zmq_hashtx = self.next_port()
        zmq_rawblock = self.next_port()
        zmq_rawtx = self.next_port()
        zmq_sequence = self.next_port()
        logfile = os.path.join(datadir, "service.log")

        cmd = [
            "bitcoind",
            "-txindex",
            "-regtest",
            "-listen=0",
            f"-port={p2p_port}",
            "-printtoconsole",
            "-server=1",
            "-fallbackfee=0.00001",
            "-minrelaytxfee=0",
            "-blockmintxfee=0",
            "-dustrelayfee=0",
            "-acceptnonstdtxn=1",
            f"-datadir={datadir}",
            f"-rpcport={rpc_port}",
            f"-rpcuser={rpc_user}",
            f"-rpcpassword={rpc_password}",
            f"-zmqpubhashblock=tcp://0.0.0.0:{zmq_hashblock}",
            f"-zmqpubhashtx=tcp://0.0.0.0:{zmq_hashtx}",
            f"-zmqpubrawblock=tcp://0.0.0.0:{zmq_rawblock}",
            f"-zmqpubrawtx=tcp://0.0.0.0:{zmq_rawtx}",
            f"-zmqpubsequence=tcp://0.0.0.0:{zmq_sequence}",
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
            "zmq_hashblock": zmq_hashblock,
            "zmq_hashtx": zmq_hashtx,
            "zmq_rawblock": zmq_rawblock,
            "zmq_rawtx": zmq_rawtx,
            "zmq_sequence": zmq_sequence,
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
