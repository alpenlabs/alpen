"""
Strata node factory.
Creates Strata sequencer and fullnode instances.
"""

import os
from pathlib import Path

import flexitest

from common.config import BitcoindConfig, ClientConfig, StrataConfig
from common.params import Params, RollupParams
from common.rpc import JsonRpcClient
from common.service import ServiceWrapper


class StrataFactory(flexitest.Factory):
    """
    Factory for creating Strata nodes.

    Command: strata -c config.toml --sequencer --datadir /path --rpc-port 9944
    """

    def __init__(self, port_range: range):
        super().__init__(list(port_range))

    @flexitest.with_ectx("ctx")
    def create_node(
        self,
        bconfig: BitcoindConfig,
        is_sequencer: bool = True,
        config_overrides: dict = dict(),
        **kwargs,
    ) -> ServiceWrapper:
        """
        Create a Strata node.

        Args:
            config: Strata configuration
            ctx: Environment context
            is_sequencer: True for sequencer, False for fullnode
            config_overrides: Additional config overrides (-o flag)
        """
        ctx = kwargs["ctx"]  # Ensured by `with_ectx` decorator.
        mode = "sequencer" if is_sequencer else "fullnode"
        datadir = Path(ctx.make_service_dir(f"strata_{mode}"))
        rpc_port = self.next_port()
        rpc_host = "127.0.0.1"
        logfile = datadir / "service.log"

        # Create config
        client_config = ClientConfig(rpc_host=rpc_host, rpc_port=rpc_port)
        config = StrataConfig(bitcoind=bconfig, client=client_config)
        config_path = datadir / "config.toml"
        with open(config_path, "w") as f:
            f.write(config.as_toml_string())

        # Create rollup params
        params = RollupParams()  # Initialize with default values
        params_path = datadir / "rollup-params.json"
        with open(params_path, "w") as f:
            f.write(params.as_json_string())

        # Build command
        cmd = [
            "strata",
            "-c",
            config_path,
            "--datadir",
            str(datadir),
            "--rollup-params",
            str(params_path),
            "--rpc-host",
            rpc_host,
            "--rpc-port",
            str(rpc_port),
        ]

        if is_sequencer:
            cmd.append("--sequencer")

        # Add config overrides
        if config_overrides:
            for key, value in config_overrides.items():
                cmd.extend(["-o", f"{key}={value}"])

        rpc_url = f"http://{rpc_host}:{rpc_port}"

        props = {
            "rpc_port": rpc_port,
            "rpc_host": rpc_host,
            "rpc_url": rpc_url,
            "datadir": datadir,
            "mode": mode,
        }

        def make_rpc() -> JsonRpcClient:
            return JsonRpcClient(rpc_url)

        svc = ServiceWrapper(
            props, cmd, stdout=str(logfile), rpc_factory=make_rpc, name=f"strata_{mode}"
        )
        svc.stop_timeout = 30
        svc.start()

        return svc
