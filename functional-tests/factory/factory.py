import contextlib
import logging
import os
import pty
import re
import shutil
import subprocess
from subprocess import CalledProcessError
from types import SimpleNamespace
from typing import Optional

import flexitest
import web3
import web3.middleware
from bitcoinlib.services.bitcoind import BitcoindClient

from factory import seqrpc
from factory.config import (
    BitcoindConfig,
    ClientConfig,
    Config,
    ExecConfig,
    RethELConfig,
)
from load.cfg import LoadConfig
from load.service import LoadGeneratorService
from utils.constants import BD_PASSWORD, BD_USERNAME
from utils.utils import ProverClientSettings


class BitcoinFactory(flexitest.Factory):
    def __init__(self, port_range: list[int]):
        super().__init__(port_range)

    @flexitest.with_ectx("ctx")
    def create_regtest_bitcoin(self, ctx: flexitest.EnvContext) -> flexitest.Service:
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
            f"-datadir={datadir}",
            f"-rpcport={rpc_port}",
            f"-rpcuser={BD_USERNAME}",
            f"-rpcpassword={BD_PASSWORD}",
        ]

        props = {
            "p2p_port": p2p_port,
            "rpc_port": rpc_port,
            "rpc_user": BD_USERNAME,
            "rpc_password": BD_PASSWORD,
            "walletname": "testwallet",
        }

        svc = flexitest.service.ProcService(props, cmd, stdout=logfile)
        svc.start()

        def _create_rpc() -> BitcoindClient:
            st = svc.check_status()
            if not st:
                raise RuntimeError("service isn't active")
            url = f"http://{BD_USERNAME}:{BD_PASSWORD}@localhost:{rpc_port}"
            return BitcoindClient(base_url=url, network="regtest")

        svc.create_rpc = _create_rpc

        return svc


class StrataFactory(flexitest.Factory):
    def __init__(self, port_range: list[int]):
        super().__init__(port_range)

    @flexitest.with_ectx("ctx")
    def create_sequencer_node(
        self,
        bitcoind_config: BitcoindConfig,
        reth_config: RethELConfig,
        sequencer_address: str,  # TODO: remove this
        rollup_params: str,
        ctx: flexitest.EnvContext,
        multi_instance_enabled: bool = False,
        name_suffix: str = "",
        instance_id: int = 0,
    ) -> flexitest.Service:
        if multi_instance_enabled:
            datadir = ctx.make_service_dir(f"sequencer.{instance_id}.{name_suffix}")
        else:
            datadir = ctx.make_service_dir("sequencer")
        rpc_port = self.next_port()
        rpc_host = "127.0.0.1"
        logfile = os.path.join(datadir, "service.log")

        # Write rollup params to file
        rollup_params_file = os.path.join(datadir, "rollup_params.json")
        with open(rollup_params_file, "w") as f:
            f.write(rollup_params)

        # Create config
        config = Config(
            bitcoind=bitcoind_config,
            exec=ExecConfig(reth=reth_config),
        )

        # Also write config as toml
        config_file = os.path.join(datadir, "config.toml")
        with open(config_file, "w") as f:
            f.write(config.as_toml_string())

        # fmt: off
        cmd = [
            "strata-client",
            "--datadir", datadir,
            "--config", config_file,
            "--rollup-params", rollup_params_file,
            "--rpc-host", rpc_host,
            "--rpc-port", str(rpc_port),

            "--sequencer"
        ]
        # fmt: on

        rpc_url = f"ws://{rpc_host}:{rpc_port}"
        props = {
            "rpc_host": rpc_host,
            "rpc_port": rpc_port,
            "rpc_url": rpc_url,
            "address": sequencer_address,
        }

        svc = flexitest.service.ProcService(props, cmd, stdout=logfile)
        svc.stop_timeout = 30
        svc.start()
        _inject_service_create_rpc(svc, rpc_url, "sequencer")

        def _datadir_path() -> str:
            return datadir

        def snapshot_dir_path(idx: int):
            return f"{datadir}.{idx}"

        def _snapshot_datadir(idx: int):
            snapshot_dir = snapshot_dir_path(idx)
            os.makedirs(snapshot_dir, exist_ok=True)
            shutil.copytree(datadir, snapshot_dir, dirs_exist_ok=True)

        def _restore_snapshot(idx: int):
            assert not svc.is_started(), "Should call restore only when service is stopped"
            snapshot_dir = snapshot_dir_path(idx)
            assert os.path.exists(snapshot_dir)
            os.rename(datadir, f"{datadir}.b.{idx}")
            os.rename(snapshot_dir, datadir)

        svc.snapshot_datadir = _snapshot_datadir
        svc.restore_snapshot = _restore_snapshot
        svc.datadir_path = _datadir_path

        return svc


class StrataSequencerFactory(flexitest.Factory):
    def __init__(self):
        super().__init__([])

    @flexitest.with_ectx("ctx")
    def create_sequencer_signer(
        self,
        sequencer_rpc_host: str,
        sequencer_rpc_port: str,
        ctx: flexitest.EnvContext,
        epoch_gas_limit: Optional[int] = None,
        multi_instance_enabled: bool = False,
        instance_id: int = 0,
        name_suffix: str = "",
    ) -> flexitest.Service:
        if multi_instance_enabled:
            datadir = ctx.make_service_dir(f"sequencer_signer.{instance_id}.{name_suffix}")
        else:
            datadir = ctx.make_service_dir("sequencer_signer")

        seqkey_path = os.path.join(ctx.envdd_path, "_init", "seqkey.bin")
        logfile = os.path.join(datadir, "service.log")

        # fmt: off
        cmd = [
            "strata-sequencer-client",
            "--sequencer-key", seqkey_path,
            "--rpc-host", sequencer_rpc_host,
            "--rpc-port", str(sequencer_rpc_port),
        ]
        # fmt: on

        if epoch_gas_limit is not None:
            cmd.extend(["--epoch-gas-limit", str(epoch_gas_limit)])

        props = {
            "seqkey": seqkey_path,
        }
        svc = flexitest.service.ProcService(props, cmd, stdout=logfile)
        svc.stop_timeout = 30
        svc.start()

        return svc


# TODO merge with `StrataFactory` to reuse most of the init steps
class FullNodeFactory(flexitest.Factory):
    def __init__(self, port_range: list[int]):
        super().__init__(port_range)
        self._next_idx = 1

    def next_idx(self) -> int:
        idx = self._next_idx
        self._next_idx += 1
        return idx

    @flexitest.with_ectx("ctx")
    def create_fullnode(
        self,
        bitcoind_config: BitcoindConfig,
        reth_config: RethELConfig,
        sequencer_rpc: str,
        rollup_params: str,
        ctx: flexitest.EnvContext,
        name_suffix: str = "",
    ) -> flexitest.Service:
        idx = self.next_idx()

        name = f"fullnode.{idx}.{name_suffix}" if name_suffix != "" else f"fullnode.{idx}"

        datadir = ctx.make_service_dir(name)
        rpc_host = "127.0.0.1"
        rpc_port = self.next_port()
        logfile = os.path.join(datadir, "service.log")

        rollup_params_file = os.path.join(datadir, "rollup_params.json")
        with open(rollup_params_file, "w") as f:
            f.write(rollup_params)

        # Create config
        config = Config(
            bitcoind=bitcoind_config,
            client=ClientConfig(sync_endpoint=sequencer_rpc),
            exec=ExecConfig(reth=reth_config),
        )

        # Also write config as toml
        config_file = os.path.join(datadir, "config.toml")
        with open(config_file, "w") as f:
            f.write(config.as_toml_string())

        # fmt: off
        cmd = [
            "strata-client",
            "--datadir", datadir,
            "--config", config_file,
            "--rollup-params", rollup_params_file,
            "--rpc-host", rpc_host,
            "--rpc-port", str(rpc_port),
        ]
        # fmt: on

        rpc_url = f"ws://localhost:{rpc_port}"
        props = {
            "id": idx,
            "rpc_port": rpc_port,
            "rpc_url": rpc_url,
        }

        svc = flexitest.service.ProcService(props, cmd, stdout=logfile)
        svc.stop_timeout = 30
        svc.start()
        _inject_service_create_rpc(svc, rpc_url, name)
        return svc


class RethFactory(flexitest.Factory):
    def __init__(self, port_range: list[int]):
        super().__init__(port_range)

    @flexitest.with_ectx("ctx")
    def create_exec_client(
        self,
        id: int,
        reth_secret_path: str,
        sequencer_reth_rpc: Optional[str],
        ctx: flexitest.EnvContext,
        custom_chain: str = "dev",
        name_suffix: str = "",
        enable_state_diff_gen: bool = False,
    ) -> flexitest.Service:
        name = f"reth.{id}{'.' + name_suffix if name_suffix else ''}"
        datadir = ctx.make_service_dir(name)
        authrpc_port = self.next_port()
        listener_port = self.next_port()
        ethrpc_ws_port = self.next_port()
        ethrpc_http_port = self.next_port()
        logfile = os.path.join(datadir, "service.log")

        # fmt: off
        cmd = [
            "alpen-reth",
            "--disable-discovery",
            "--ipcdisable",
            "--datadir", datadir,
            "--authrpc.port", str(authrpc_port),
            "--authrpc.jwtsecret", reth_secret_path,
            "--port", str(listener_port),
            "--ws",
            "--ws.port", str(ethrpc_ws_port),
            "--http",
            "--http.port", str(ethrpc_http_port),
            "--color", "never",
            "--enable-witness-gen",
            "--custom-chain", custom_chain,
            "-vvvv"
        ]
        # fmt: on

        # Right now, exex pipeline seems to be very slow and suboptimal.
        # Disabling state_diff exex for now for the basic env, with the
        # option to enable in a separate `state_diffs` env.
        # TODO(STR-1381): investigate and optimize exex.
        if enable_state_diff_gen:
            cmd.append(
                "--enable-state-diff-gen",
            )

        if sequencer_reth_rpc is not None:
            cmd.extend(["--sequencer-http", sequencer_reth_rpc])

        props = {"rpc_port": authrpc_port, "eth_rpc_http_port": ethrpc_http_port}

        ethrpc_url = f"ws://localhost:{ethrpc_ws_port}"

        svc = flexitest.service.ProcService(props, cmd, stdout=logfile)
        svc.stop_timeout = 30
        svc.start()

        def _create_web3():
            http_ethrpc_url = f"http://localhost:{ethrpc_http_port}"
            w3 = web3.Web3(web3.Web3.HTTPProvider(http_ethrpc_url))
            # address, pk hardcoded in test genesis config
            w3.address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
            account = w3.eth.account.from_key(
                "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
            )
            w3.middleware_onion.add(web3.middleware.SignAndSendRawMiddlewareBuilder.build(account))
            return w3

        def snapshot_dir_path(idx: int):
            return os.path.join(ctx.envdd_path, f"reth.{id}.{idx}")

        def _snapshot_datadir(idx: int):
            snapshot_dir = snapshot_dir_path(idx)
            os.makedirs(snapshot_dir, exist_ok=True)
            shutil.copytree(datadir, snapshot_dir, dirs_exist_ok=True)

        def _restore_snapshot(idx: int):
            assert not svc.is_started(), "Should call restore only when service is stopped"
            snapshot_dir = snapshot_dir_path(idx)
            assert os.path.exists(snapshot_dir)
            shutil.rmtree(datadir)
            os.rename(snapshot_dir, datadir)

        _inject_service_create_rpc(svc, ethrpc_url, name)
        svc.create_web3 = _create_web3
        svc.snapshot_datadir = _snapshot_datadir
        svc.restore_snapshot = _restore_snapshot

        return svc


class ProverClientFactory(flexitest.Factory):
    def __init__(self, port_range: list[int]):
        super().__init__(port_range)

    @flexitest.with_ectx("ctx")
    def create_prover_client(
        self,
        bitcoind_config: BitcoindConfig,
        sequencer_url: str,
        reth_url: str,
        rollup_params: str,
        settings: ProverClientSettings,
        ctx: flexitest.EnvContext,
        name_suffix: str = "",
    ):
        name = f"prover_client.{name_suffix}" if name_suffix != "" else "prover_client"

        datadir = ctx.make_service_dir(name)
        logfile = os.path.join(datadir, "service.log")

        rpc_port = self.next_port()
        rpc_url = f"ws://localhost:{rpc_port}"

        rollup_params_file = os.path.join(datadir, "rollup_params.json")
        with open(rollup_params_file, "w") as f:
            f.write(rollup_params)

        # Create TOML configuration file for prover-client
        config_file = os.path.join(datadir, "prover-client.toml")
        config_content = f"""# Prover Client Configuration for functional test
# Generated automatically by functional test factory

[rpc]
# RPC server configuration for development mode
dev_port = {rpc_port}
dev_url = "0.0.0.0"

[workers]
# Number of worker threads for different proving backends
native = {settings.native_workers}
sp1 = 20
risc0 = 20

[timing]
# Polling and timing configuration (in milliseconds and seconds)
polling_interval_ms = {settings.polling_interval}
checkpoint_poll_interval_s = 1

[retry]
# Retry policy configuration
max_retry_counter = {settings.max_retry_counter}
bitcoin_retry_count = 3
bitcoin_retry_interval_ms = 1000

[features]
# Feature flags to enable/disable functionality
enable_dev_rpcs = true
enable_checkpoint_runner = {str(settings.enable_checkpoint_proving).lower()}
"""

        with open(config_file, "w") as f:
            f.write(config_content)

        # fmt: off
        cmd = [
            "strata-prover-client",
            "--config", config_file,
            "--sequencer-rpc", sequencer_url,
            "--reth-rpc", reth_url,
            "--rollup-params", rollup_params_file,
            "--bitcoind-url", bitcoind_config.rpc_url,
            "--bitcoind-user", bitcoind_config.rpc_user,
            "--bitcoind-password", bitcoind_config.rpc_password,
            "--datadir", datadir,
        ]
        # fmt: on

        props = {"rpc_port": rpc_port}

        svc = flexitest.service.ProcService(props, cmd, stdout=logfile)
        svc.stop_timeout = 30
        svc.start()
        _inject_service_create_rpc(svc, rpc_url, "prover")
        return svc


class LoadGeneratorFactory(flexitest.Factory):
    def __init__(self, port_range: list[int]):
        super().__init__(port_range)

    @flexitest.with_ectx("ctx")
    def create_simple_loadgen(
        self,
        load_cfg: LoadConfig,
        ctx: flexitest.EnvContext,
    ) -> flexitest.Service:
        name = "load_generator"

        datadir = ctx.make_service_dir(name)
        rpc_port = self.next_port()

        rpc_url = f"ws://localhost:{rpc_port}"

        svc = LoadGeneratorService(datadir, load_cfg)
        svc.start()
        _inject_service_create_rpc(svc, rpc_url, name)
        return svc


class AlpenCliFactory(flexitest.Factory):
    def __init__(self):
        # doesn't require any ports
        super().__init__([])

    def run_tty(
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
        assert self.svc is not None, "service not initialized"
        assert self.config_file is not None, "config path not set"

        result = self.run_tty(
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

    def _check_config(self) -> bool:
        # fmt: off
        cmd = [
            "alpen",
            "config",
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, self.config_file) == self.config_file

    def _scan(self) -> Optional[str]:
        cmd = [
            # fmt: off
            "alpen",
            "scan",
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"Scan complete")

    def _l2_balance(self) -> Optional[str]:
        # fmt: off
        cmd = [
            "alpen",
            "balance",
            "alpen"
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"^Total:\s+([0-9]+(?:\.[0-9]+)?)\s+BTC\b")

    def _l1_balance(self) -> Optional[str]:
        # fmt: off
        cmd = [
            "alpen",
            "balance",
            "signet"
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"^Total:\s+([0-9]+(?:\.[0-9]+)?)\s+BTC\b")

    def _l2_address(self) -> Optional[str]:
        # fmt: off
        cmd = [
            "alpen",
            "receive",
            "alpen"
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"0x[0-9a-fA-F]{40}")

    def _l1_address(self):
        # fmt: off
        cmd = [
            "alpen",
            "receive",
            "signet"
        ]

        # fmt: on
        return self._run_and_extract_with_re(cmd, r"\b(?:bc1|tb1|bcrt1)[0-9a-z]{25,59}\b")

    def _deposit(self) -> Optional[str]:
        # fmt: off
        cmd = [
            "alpen",
            "deposit",
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"Transaction ID:\s*([0-9a-f]{64})")

    def _withdraw(self):
        # fmt: off
        cmd = [
            "alpen",
            "withdraw",
        ]
        # fmt: on
        return self._run_and_extract_with_re(cmd, r"Transaction ID:\s*(0x[0-9a-f]{64})")

    @flexitest.with_ectx("ctx")
    def setup_environment(
        self,
        reth_endpoint: str,
        bitcoin_config: BitcoindConfig,
        pubkey: str,
        magic_bytes: str,
        ctx: flexitest.EnvContext,
    ) -> flexitest.Service:
        name = "alpen_cli"

        self.datadir = ctx.make_service_dir(name)
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

        # create a Service like object, but not really a service
        self.svc = SimpleNamespace()
        self.svc.l2_address = lambda: self._l2_address()
        self.svc.l1_address = lambda: self._l1_address()
        self.svc.scan = lambda: self._scan()
        self.svc.l2_balance = lambda: self._l2_balance()
        self.svc.l1_balance = lambda: self._l1_balance()
        self.svc.deposit = lambda: self._deposit()
        self.svc.withdraw = lambda: self._withdraw()
        self.svc.is_started = lambda: False

        assert self._check_config(), "config file path should match"
        return self.svc


def _inject_service_create_rpc(svc: flexitest.service.ProcService, rpc_url: str, name: str):
    """
    Injects a `create_rpc` method using JSON-RPC onto a `ProcService`, checking
    its status before each call.
    """

    def _status_ck(method: str):
        """
        Hook to check that the process is still running before every call.
        """
        if not svc.check_status():
            logging.warning(f"service '{name}' seems to have crashed as of call to {method}")
            raise RuntimeError(f"process '{name}' crashed")

    def _create_rpc() -> seqrpc.JsonrpcClient:
        rpc = seqrpc.JsonrpcClient(rpc_url)
        rpc._pre_call_hook = _status_ck
        return rpc

    svc.create_rpc = _create_rpc
