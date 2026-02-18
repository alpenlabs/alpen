"""
Strata node factory.
Creates Strata sequencer and fullnode instances.
"""

import contextlib
import shutil
import subprocess
from pathlib import Path

import flexitest

from common.config import (
    BitcoindConfig,
    ClientConfig,
    EpochSealingConfig,
    OLParams,
    RollupParams,
    SequencerConfig,
    ServiceType,
    StrataConfig,
)
from common.config.params import GenesisL1View
from common.services import StrataProps, StrataService


class StrataFactory(flexitest.Factory):
    """
    Factory for creating Strata nodes.
    """

    def _ensure_sequencer_key(self, path: Path) -> None:
        if path.exists():
            return

        tool = shutil.which("strata-datatool")
        if tool is not None:
            cmd = [
                tool,
                "-b",
                "regtest",
                "genxpriv",
                "-f",
                str(path),
            ]
            res = subprocess.run(cmd, capture_output=True, text=True)
            if res.returncode != 0:
                details = res.stderr.strip() or res.stdout.strip()
                raise RuntimeError(f"Failed to generate sequencer key: {details}")
            return

        # Fallback: deterministic testnet/regtest xpriv used for tests.
        # Keep this in sync with known-good test vectors to avoid dependency on binaries.
        path.write_text(
            "tprv8ZgxMBicQKsPd4arFr7sKjSnKFDVMR2JHw9Y8L9nXN4kiok4u28LpHijEudH3mMYoL4pM5UL9Bgdz2M4Cy8EzfErmU9m86ZTw6hCzvFeTg7"
        )

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
        config = StrataConfig(
            bitcoind=bconfig,
            client=client_config,
            sequencer=sequencer_config,
            epoch_sealing=epoch_sealing,
        )
        config_path = datadir / "config.toml"
        with open(config_path, "w") as f:
            f.write(config.as_toml_string())

        # Create rollup params
        params = RollupParams().with_genesis_l1(genesis_l1)
        params_path = datadir / "rollup-params.json"
        with open(params_path, "w") as f:
            f.write(params.as_json_string())

        # Create OL params (use provided or create default)
        if ol_params is None:
            ol_params = OLParams().with_genesis_l1(genesis_l1)
        ol_params_path = datadir / "ol-params.json"
        with open(ol_params_path, "w") as f:
            f.write(ol_params.as_json_string())

        # Build command
        cmd = [
            "strata",
            "-c",
            str(config_path),
            "--datadir",
            str(datadir),
            "--rollup-params",
            str(params_path),
            "--ol-params",
            str(ol_params_path),
            "--rpc-host",
            rpc_host,
            "--rpc-port",
            str(rpc_port),
        ]

        if is_sequencer:
            sequencer_key_path = datadir / "sequencer_root_key"
            self._ensure_sequencer_key(sequencer_key_path)
            cmd.extend(["--sequencer", "--sequencer-key", str(sequencer_key_path)])

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
