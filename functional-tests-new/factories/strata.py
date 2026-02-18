"""
Strata node factory.
Creates Strata sequencer and fullnode instances.
"""

import contextlib
from pathlib import Path

import flexitest

from common.config import (
    BitcoindConfig,
    ClientConfig,
    EpochSealingConfig,
    OLParams,
    SequencerConfig,
    ServiceType,
    StrataConfig,
)
from common.config.params import GenesisL1View
from common.datatool import (
    generate_asm_params,
    generate_ol_params,
    generate_rollup_params,
)
from common.services import StrataProps, StrataService


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
        genesis_l1: GenesisL1View,
        is_sequencer: bool = True,
        config_overrides: dict | None = None,
        ol_params: OLParams | None = None,
        epoch_sealing: EpochSealingConfig | None = None,
        **kwargs,
    ) -> StrataService:
        """
        Create a Strata node.

        Args:
            bconfig: Bitcoin daemon configuration
            is_sequencer: True for sequencer, False for fullnode
            config_overrides: Additional config overrides (-o flag)
            ol_params: Custom OL parameters (genesis accounts, etc.)
            epoch_sealing: Epoch sealing config (controls terminal block frequency)
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
        sequencer_config = SequencerConfig() if is_sequencer else None
        config_kwargs = dict(
            bitcoind=bconfig,
            client=client_config,
            sequencer=sequencer_config,
        )
        if epoch_sealing is not None:
            config_kwargs["epoch_sealing"] = epoch_sealing
        config = StrataConfig(**config_kwargs)
        config_path = datadir / "config.toml"
        with open(config_path, "w") as f:
            f.write(config.as_toml_string())

        genesis_l1_height = genesis_l1.blk.height

        # Generate rollup params via datatool.
        params_data = generate_rollup_params(datadir, bconfig, genesis_l1_height)

        # Generate OL params via datatool (uses Bitcoin RPC to fetch genesis L1 block).
        ol_params_path = generate_ol_params(datadir, bconfig, genesis_l1_height)

        # Generate ASM params via datatool (computes correct genesis_ol_blkid from OL params).
        asm_params_path = generate_asm_params(
            datadir,
            bconfig,
            genesis_l1_height,
            params_data.operator_keys,
            ol_params_path=ol_params_path,
        )

        # Build command
        cmd = [
            "strata",
            "-c",
            str(config_path),
            "--datadir",
            str(datadir),
            "--rollup-params",
            str(params_data.params_path),
            "--ol-params",
            str(ol_params_path),
            "--asm-params",
            str(asm_params_path),
            "--rpc-host",
            rpc_host,
            "--rpc-port",
            str(rpc_port),
        ]

        if is_sequencer:
            cmd.extend(["--sequencer", "--sequencer-key", str(params_data.sequencer_key_path)])

        # Add config overrides
        if config_overrides:
            for key, value in config_overrides.items():
                cmd.extend(["-o", f"{key}={value}"])

        rpc_url = f"http://{rpc_host}:{rpc_port}"

        props: StrataProps = {
            "rpc_port": rpc_port,
            "rpc_host": rpc_host,
            "rpc_url": rpc_url,
            "datadir": str(datadir),
            "mode": mode,
        }

        svc = StrataService(
            props,
            cmd,
            stdout=str(logfile),
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
