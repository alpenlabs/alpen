"""
Strata node factory.
Creates Strata sequencer and fullnode instances.
"""

import contextlib
from pathlib import Path

import flexitest

from common.config import BitcoindConfig, ClientConfig, RollupParams, ServiceType, StrataConfig
from common.rpc import JsonRpcClient
from common.services import StrataServiceWrapper


class StrataFactory(flexitest.Factory):
    """
    Factory for creating Strata nodes.
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
    def create_node(
        self,
        bconfig: BitcoindConfig,
        is_sequencer: bool = True,
        config_overrides: dict | None = None,
        **kwargs,
    ) -> StrataServiceWrapper:
        """
        Create a Strata node.

        Args:
            bconfig: Bitcoin daemon configuration
            is_sequencer: True for sequencer, False for fullnode
            config_overrides: Additional config overrides (-o flag)
        """
        # Ensured by `with_ectx` decorator. Don't like this though.
        ctx: flexitest.EnvContext = kwargs["ctx"]

        if config_overrides is None:
            config_overrides = dict()

        mode = "sequencer" if is_sequencer else "fullnode"
        datadir = Path(ctx.make_service_dir(f"{ServiceType.Strata}_{mode}"))
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
            str(config_path),
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

        props = dict(
            rpc_port=rpc_port,
            rpc_host=rpc_host,
            rpc_url=rpc_url,
            datadir=datadir,
            mode=mode,
        )

        def make_rpc() -> JsonRpcClient:
            return JsonRpcClient(rpc_url)

        svc = StrataServiceWrapper(
            props,
            cmd,
            stdout=str(logfile),
            rpc_factory=make_rpc,
            name=f"{ServiceType.Strata}_{mode}",
        )
        svc.stop_timeout = 30
        try:
            svc.start()
        except Exception as e:
            # Ensure cleanup on failure to prevent resource leaks
            with contextlib.suppress(Exception):
                svc.stop()
            raise RuntimeError(f"Failed to start strata service ({mode}): {e}") from e

        return svc
