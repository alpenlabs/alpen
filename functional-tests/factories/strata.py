"""
Strata node factory.
Creates Strata sequencer and fullnode instances.
"""

import contextlib
from pathlib import Path
from typing import NamedTuple

import flexitest

from common.config import (
    BitcoindConfig,
    ClientConfig,
    EpochSealingConfig,
    OLParams,
    SequencerConfig,
    SequencerRuntimeConfig,
    ServiceType,
    StrataConfig,
)
from common.datatool import (
    generate_asm_params,
    generate_ol_params,
    generate_rollup_params,
    generate_rollup_params_unchecked,
)
from common.services import StrataProps, StrataService


class CreateNodeResult(NamedTuple):
    """Result of creating a Strata node."""

    service: StrataService
    sequencer_key_path: Path | None


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
        genesis_l1_height: int,
        is_sequencer: bool = True,
        config_overrides: dict[str, object] | None = None,
        ol_params: OLParams | None = None,
        epoch_sealing_config: EpochSealingConfig | None = None,
        use_unchecked_cred_rule: bool = False,
        admin_confirmation_depth: int | None = None,
        **kwargs,
    ) -> CreateNodeResult:
        """
        Create a Strata node.

        Args:
            bconfig: Bitcoin daemon configuration
            genesis_l1_height: Genesis L1 height used for param generation.
            is_sequencer: True for sequencer, False for fullnode
            config_overrides: Additional config overrides (-o flag)
            ol_params: Custom OL parameters (genesis accounts, etc.)
            epoch_sealing_config: Epoch sealing config for TOML. Default used if None.
            use_unchecked_cred_rule: If True, generates params with CredRule::Unchecked.
            admin_confirmation_depth: Optional admin subprotocol confirmation depth.
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
        config = StrataConfig(
            bitcoind=bconfig,
            client=client_config,
        )
        config_path = datadir / "config.toml"
        with open(config_path, "w") as f:
            f.write(config.as_toml_string())

        sequencer_config_path = datadir / "sequencer.toml"
        if is_sequencer:
            sequencer_runtime_config = SequencerRuntimeConfig(
                sequencer=SequencerConfig(),
                epoch_sealing=epoch_sealing_config,
            )
            with open(sequencer_config_path, "w") as f:
                f.write(sequencer_runtime_config.as_toml_string())

        # Generate rollup params via datatool (also produces keys used below).
        if use_unchecked_cred_rule:
            params_data = generate_rollup_params_unchecked(datadir, bconfig, genesis_l1_height)
        else:
            params_data = generate_rollup_params(datadir, bconfig, genesis_l1_height)

        # Generate or write OL params.
        if ol_params is not None:
            ol_params_path = datadir / "ol-params.json"
            ol_params_path.write_text(ol_params.as_json_string())
        else:
            ol_params_path = generate_ol_params(datadir, bconfig, genesis_l1_height)

        # Generate ASM params via datatool (computes correct genesis_ol_blkid from OL params).
        asm_params_path = generate_asm_params(
            datadir,
            bconfig,
            genesis_l1_height,
            params_data.operator_keys,
            ol_params_path=ol_params_path,
            admin_confirmation_depth=admin_confirmation_depth,
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
            cmd.extend(
                [
                    "--sequencer",
                    "--sequencer-config",
                    str(sequencer_config_path),
                ]
            )

        # Add config overrides
        if config_overrides:
            for key, value in config_overrides.items():
                cmd.extend(["-o", f"{key}={value}"])

        rpc_url = f"http://{rpc_host}:{rpc_port}"

        resolved_slots_per_epoch = 4
        if epoch_sealing_config is not None and epoch_sealing_config.slots_per_epoch is not None:
            resolved_slots_per_epoch = epoch_sealing_config.slots_per_epoch

        props: StrataProps = {
            "rpc_port": rpc_port,
            "rpc_host": rpc_host,
            "rpc_url": rpc_url,
            "datadir": str(datadir),
            "mode": mode,
            "slots_per_epoch": resolved_slots_per_epoch,
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

        seq_key_path = params_data.sequencer_key_path if is_sequencer else None
        return CreateNodeResult(svc, seq_key_path)
