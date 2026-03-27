"""
Strata-signer factory.

Creates strata-signer instances that connect to a running strata node
and handle signing duties.
"""

import contextlib
import shutil
from pathlib import Path

import flexitest

from common.config import ServiceType
from common.services.signer import SignerProps, SignerService

# Poll interval in milliseconds for functional tests (faster than production default).
TEST_POLL_INTERVAL_MS = 1_000


class SignerFactory(flexitest.Factory):
    """Factory for creating strata-signer instances."""

    def __init__(self, port_range: range):
        super().__init__(list(port_range))

    @flexitest.with_ectx("ctx")
    def create_signer(
        self,
        sequencer_key_path: Path,
        rpc_host: str,
        rpc_port: int,
        **kwargs,
    ) -> SignerService:
        """
        Create a strata-signer instance.

        Args:
            sequencer_key_path: Path to the sequencer root key file (xprv).
            rpc_host: Host of the strata node RPC server.
            rpc_port: Port of the strata node RPC server.
        """
        ctx: flexitest.EnvContext = kwargs["ctx"]

        datadir = Path(ctx.make_service_dir(str(ServiceType.StrataSigner)))
        logfile = datadir / "service.log"
        ws_url = f"ws://{rpc_host}:{rpc_port}"

        # Write signer config TOML
        config_path = datadir / "signer-config.toml"
        config_path.write_text(
            f'sequencer_key = "{sequencer_key_path}"\n'
            f'sequencer_endpoint = "{ws_url}"\n'
            f"duty_poll_interval = {TEST_POLL_INTERVAL_MS}\n"
        )

        tool = shutil.which("strata-signer")
        if tool is None:
            raise RuntimeError("strata-signer not found on PATH")

        cmd = [tool, "-c", str(config_path)]

        props: SignerProps = {
            "datadir": str(datadir),
        }

        svc = SignerService(props, cmd, stdout=str(logfile), name=str(ServiceType.StrataSigner))
        svc.stop_timeout = 10
        try:
            svc.start()
        except Exception as e:
            with contextlib.suppress(Exception):
                svc.stop()
            raise RuntimeError(f"Failed to start strata-signer: {e}") from e

        return svc
