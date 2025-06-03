import logging
import os
import shutil
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
from utils import *
from utils.constants import *


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

        def _create_rpc():
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
        svc.start()
        _inject_service_create_rpc(svc, rpc_url, "sequencer")
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
            "--enable-state-diff-gen",
            "--custom-chain", custom_chain,
            "-vvvv"
        ]
        # fmt: on

        if sequencer_reth_rpc is not None:
            cmd.extend(["--sequencer-http", sequencer_reth_rpc])

        props = {"rpc_port": authrpc_port, "eth_rpc_http_port": ethrpc_http_port}

        ethrpc_url = f"ws://localhost:{ethrpc_ws_port}"

        svc = flexitest.service.ProcService(props, cmd, stdout=logfile)
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
checkpoint_poll_interval_s = 10

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

    def _create_rpc():
        rpc = seqrpc.JsonrpcClient(rpc_url)
        rpc._pre_call_hook = _status_ck
        return rpc

    svc.create_rpc = _create_rpc
