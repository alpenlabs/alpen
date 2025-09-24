import os
import re
from subprocess import CalledProcessError

from envs.env_control_builder import EnvControlBuilder, ServiceNotAvailable
from utils import run_tty


class AlpenCli:
    """
    Alpen CLI client for functional tests.
    Requires client to be built with "test-mode" cargo feature.
    """

    def __init__(self, config_file: str, datadir: str):
        self.config_file = config_file
        self.datadir = datadir

    def _run_and_extract_with_re(self, cmd, re_pattern) -> str | None:
        assert self.config_file is not None, "config path not set"

        result = run_tty(
            cmd,
            capture_output=True,
            env={"CLI_CONFIG": self.config_file, "PROJ_DIRS": self.datadir},
        )
        try:
            result.check_returncode()
        except CalledProcessError:
            return None

        output = result.stdout.decode("utf-8")
        m = re.search(re_pattern, output)
        if not m:
            return None
        return m.group(1) if m.lastindex else m.group(0)

    def check_config(self) -> bool:
        # fmt: off
        cmd = [
            "alpen",
            "config",
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, self.config_file) == self.config_file

    def scan(self) -> str | None:
        cmd = [
            # fmt: off
            "alpen",
            "scan",
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"Scan complete")

    def l2_balance(self) -> str | None:
        # fmt: off
        cmd = [
            "alpen",
            "balance",
            "alpen"
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"^Total:\s+([0-9]+(?:\.[0-9]+)?)\s+BTC\b")

    def l1_balance(self) -> str | None:
        # fmt: off
        cmd = [
            "alpen",
            "balance",
            "signet"
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"^Total:\s+([0-9]+(?:\.[0-9]+)?)\s+BTC\b")

    def l2_address(self) -> str | None:
        # fmt: off
        cmd = [
            "alpen",
            "receive",
            "alpen"
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"0x[0-9a-fA-F]{40}")

    def l1_address(self):
        # fmt: off
        cmd = [
            "alpen",
            "receive",
            "signet"
        ]

        # fmt: on
        return self._run_and_extract_with_re(cmd, r"\b(?:bc1|tb1|bcrt1)[0-9a-z]{25,59}\b")

    def deposit(self, alpen_address=None) -> str | None:
        # fmt: off
        cmd = [
            "alpen",
            "deposit",
        ]
        if alpen_address:
            cmd.append(alpen_address)
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"Transaction ID:\s*([0-9a-f]{64})")

    def withdraw(self):
        # fmt: off
        cmd = [
            "alpen",
            "withdraw",
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"Transaction ID:\s*(0x[0-9a-f]{64})")


class AlpenCliBuilder:
    """
    Builder for AlpenCli instances with configuration setup for functional tests.
    """

    def __init__(self):
        self.service_resolver = EnvControlBuilder()
        self.pubkey = None
        self.magic_bytes = None
        self.datadir = None

    def with_pubkey(self, pubkey: str):
        self.pubkey = pubkey
        return self

    def with_magic_bytes(self, magic_bytes: str):
        self.magic_bytes = magic_bytes
        return self

    def with_datadir(self, datadir: str):
        self.datadir = datadir
        return self

    def requires_service(self, service_name: str, transform_lambda):
        """Configure service requirements for building the AlpenCli"""
        self.service_resolver.requires_service(service_name, transform_lambda)
        return self

    def build(self, ctx) -> AlpenCli | None:
        """Build AlpenCli instance with resolved service configs"""
        if not self.pubkey or not self.magic_bytes or not self.datadir:
            raise ValueError("pubkey, magic_bytes, and datadir must be set before building")

        # Get resolved configs using service resolver
        try:
            resolved_configs = self.service_resolver.build(ctx)
        except ServiceNotAvailable:
            return None

        # Extract the specific configs we need
        bitcoin_config = resolved_configs["bitcoin"]
        reth_endpoint = resolved_configs["reth"]

        # Set up the CLI configuration
        name = "alpen_cli"
        path = os.path.join(self.datadir, name)
        if not os.path.exists(path):
            os.makedirs(path)
        config_file = os.path.join(self.datadir, "alpen-cli.toml")
        config_content = f"""# Alpen-cli Configuration for functional test
# Generated automatically by functional test factory
alpen_endpoint = "{reth_endpoint}"
bitcoind_rpc_endpoint = "{bitcoin_config.rpc_url}"
bitcoind_rpc_user = "{bitcoin_config.rpc_user}"
bitcoind_rpc_pw = "{bitcoin_config.rpc_password}"
faucet_endpoint = "{bitcoin_config.rpc_url}"
bridge_pubkey = "{self.pubkey}"
magic_bytes = "{self.magic_bytes}"
network = "regtest"
seed = "838d8ba290a3066abb35b663858fa839"
"""
        with open(config_file, "w") as f:
            f.write(config_content)

        # Create and return the built AlpenCli instance
        alpen_cli = AlpenCli(config_file, self.datadir)
        assert alpen_cli.check_config(), "config file path should match"
        return alpen_cli
