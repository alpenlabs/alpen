import contextlib
import os
import pty
import re
import subprocess
from subprocess import CalledProcessError
from typing import Optional

from factory.config import BitcoindConfig


class AlpenCli:
    """
    Alpen Cli client with configuration setup specifically functional tests.
    Requires client to be built with "test-mode" cargo feature
    """

    def __init__(
        self,
        reth_endpoint: str,
        bitcoin_config: BitcoindConfig,
        pubkey: str,
        magic_bytes: str,
        datadir: str,
    ):
        name = "alpen_cli"
        # create directory
        self.datadir = datadir
        path = os.path.join(self.datadir, name)
        os.makedirs(path)
        self.config_file = os.path.join(self.datadir, "alpen-cli.toml")
        config_content = f"""# Alpen-cli Configuration for functional test
# Generated automatically by functional test factory
alpen_endpoint = "{reth_endpoint}"
bitcoind_rpc_endpoint = "{bitcoin_config.rpc_url}"
bitcoind_rpc_user = "{bitcoin_config.rpc_user}"
bitcoind_rpc_pw = "{bitcoin_config.rpc_password}"
faucet_endpoint = "{bitcoin_config.rpc_url}"
bridge_pubkey = "{pubkey}"
magic_bytes = "{magic_bytes}"
network = "regtest"
seed = "838d8ba290a3066abb35b663858fa839"
"""
        with open(self.config_file, "w") as f:
            f.write(config_content)

        assert self.check_config(), "config file path should match"

    def _run_tty(
        self, cmd, *, capture_output=False, stdout=None, env=None
    ) -> subprocess.CompletedProcess:
        """
        Runs `cmd` under a PTY (so indicatif used by Alpen-cli behaves).
        Returns a CompletedProcess; stdout is bytes when captured.
        """
        if stdout is subprocess.PIPE:
            capture_output, stdout = True, None

        buf = [] if capture_output else None

        def reader(fd):
            data = os.read(fd, 4096)
            if data:
                if buf is not None:
                    buf.append(data)
                elif stdout is None:
                    os.write(1, data)  # parent stdout
                else:
                    # file-like or text stream
                    if hasattr(stdout, "buffer"):
                        stdout.buffer.write(data)
                        stdout.flush()
                    else:
                        stdout.write(data.decode("utf-8", "replace"))
                        if hasattr(stdout, "flush"):
                            stdout.flush()
            return data

        old_env = os.environ.copy()
        try:
            if env:
                os.environ.update(env)
            rc = pty.spawn(cmd, reader)
        finally:
            with contextlib.suppress(Exception):
                os.environ.clear()
                os.environ.update(old_env)

        return subprocess.CompletedProcess(
            args=cmd,
            returncode=rc,
            stdout=(b"".join(buf) if buf is not None else None),
            stderr=None,  # PTY merges stderr
        )

    def _run_and_extract_with_re(self, cmd, re_pattern) -> Optional[str]:
        assert self.config_file is not None, "config path not set"

        result = self._run_tty(
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

    def scan(self) -> Optional[str]:
        cmd = [
            # fmt: off
            "alpen",
            "scan",
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"Scan complete")

    def l2_balance(self) -> Optional[str]:
        # fmt: off
        cmd = [
            "alpen",
            "balance",
            "alpen"
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"^Total:\s+([0-9]+(?:\.[0-9]+)?)\s+BTC\b")

    def l1_balance(self) -> Optional[str]:
        # fmt: off
        cmd = [
            "alpen",
            "balance",
            "signet"
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"^Total:\s+([0-9]+(?:\.[0-9]+)?)\s+BTC\b")

    def l2_address(self) -> Optional[str]:
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

    def deposit(self) -> Optional[str]:
        # fmt: off
        cmd = [
            "alpen",
            "deposit",
        ]
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
