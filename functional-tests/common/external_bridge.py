"""Helpers for running an external strata-bridge checkout in functional tests."""

import json
import os
import re
import select
import shutil
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import requests


DEFAULT_OPERATOR_COUNT = 2
DEFAULT_BRIDGE_REPO_CANDIDATES = ("strata-bridge-current", "strata-bridge")
DEFAULT_BRIDGE_RPC_PORT = 13000
READY_PREFIX = "BRIDGE_READY "
STAGE_PREFIX = "BRIDGE_STAGE "
STARTUP_LOG_PREFIXES = (STAGE_PREFIX, "BRIDGE_ERROR ")
DEFAULT_FDB_KNOBS = (
    "-m 2GiB -M 512MiB --cache-memory 256MiB "
    "--knob-min_available_space 0 "
    "--knob-min_available_space_ratio 0 "
    "--knob-min_available_space_ratio_safety_buffer 0 "
    "--knob-target_available_space_ratio 0 "
    "--knob-available_space_ratio_cutoff 0"
)


def find_bridge_repo() -> Path:
    """Resolve the external strata-bridge checkout."""
    configured = os.environ.get("ALPEN_EXTERNAL_BRIDGE_REPO")
    if configured:
        repo = Path(configured).expanduser().resolve()
        if not (repo / "Cargo.toml").exists():
            raise RuntimeError(f"ALPEN_EXTERNAL_BRIDGE_REPO is not a Rust repo: {repo}")
        return repo

    alpen_parent = Path(__file__).resolve().parents[3]
    for name in DEFAULT_BRIDGE_REPO_CANDIDATES:
        repo = alpen_parent / name
        if (repo / "Cargo.toml").exists():
            return repo

    raise RuntimeError(
        "External strata-bridge checkout not found. Set ALPEN_EXTERNAL_BRIDGE_REPO."
    )


def load_bridge_operator_pubkeys(repo: Path, count: int = DEFAULT_OPERATOR_COUNT) -> list[str]:
    """Load MuSig2 operator public keys from the external bridge test artifacts."""
    keys_path = repo / "functional-tests/artifacts/keys.json"
    keys = json.loads(keys_path.read_text())
    if len(keys) < count:
        raise RuntimeError(f"requested {count} bridge operators, only {len(keys)} keys available")
    return [entry["MUSIG2_KEY"] for entry in keys[:count]]


def compute_bridge_aggregate_pubkey(operator_pubkeys: list[str]) -> str:
    """Compute the MuSig2 aggregate key used by `alpen deposit`."""
    result = subprocess.run(
        [
            "strata-test-cli",
            "musig-aggregate-pks",
            "--pubkeys",
            json.dumps(operator_pubkeys),
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=True,
    )
    return result.stdout.strip()


def _extract_git_rev(cargo_toml: str, package: str) -> str:
    pattern = rf"{re.escape(package)}.*rev = \"([^\"]+)\""
    match = re.search(pattern, cargo_toml)
    if match is None:
        raise RuntimeError(f"failed to extract {package} rev from external bridge Cargo.toml")
    return match.group(1)


def build_external_bridge(repo: Path) -> None:
    """Build external bridge binaries and install auxiliary binaries."""
    if os.environ.get("ALPEN_EXTERNAL_BRIDGE_BUILD", "1") == "0":
        return

    env = os.environ.copy()
    subprocess.run(
        ["cargo", "build", "--bin", "strata-bridge"],
        cwd=repo,
        env=env,
        check=True,
    )
    subprocess.run(
        ["cargo", "build", "-p", "secret-service", "--bin", "secret-service"],
        cwd=repo,
        env=env,
        check=True,
    )
    subprocess.run(["cargo", "build", "--bin", "dev-cli"], cwd=repo, env=env, check=True)

    install_root = repo / "functional-tests/_dd/.bin"
    bin_dir = install_root / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)

    cargo_toml = (repo / "Cargo.toml").read_text()
    mosaic_rev = _extract_git_rev(cargo_toml, "mosaic-rpc-api")
    asm_rev = _extract_git_rev(cargo_toml, "strata-asm-worker")

    if shutil.which("mosaic", path=str(bin_dir)) is None:
        subprocess.run(
            [
                "cargo",
                "install",
                "--locked",
                "--root",
                str(install_root),
                "--git",
                "https://github.com/alpenlabs/mosaic",
                "--rev",
                mosaic_rev,
                "mosaic",
            ],
            check=True,
        )

    if shutil.which("strata-asm-runner", path=str(bin_dir)) is None:
        subprocess.run(
            [
                "cargo",
                "install",
                "--locked",
                "--root",
                str(install_root),
                "--git",
                "https://github.com/alpenlabs/asm",
                "--rev",
                asm_rev,
                "strata-asm-runner",
            ],
            check=True,
        )


def _bridge_process_env(repo: Path) -> dict[str, str]:
    env = os.environ.copy()
    ft_root = repo / "functional-tests"
    bin_paths = [repo / "target/debug", ft_root / "_dd/.bin/bin"]
    env["PATH"] = os.pathsep.join(str(path) for path in bin_paths) + os.pathsep + env["PATH"]

    fdb_root = ft_root / "_dd/.tools/fdb-7.3.43/root"
    if fdb_root.exists():
        env.setdefault("FDBSERVER_PATH", str(fdb_root / "usr/local/libexec/fdbserver"))
        env.setdefault("FDBCLI_PATH", str(fdb_root / "usr/local/bin/fdbcli"))
        env.setdefault("FDB_LIBRARY_PATH", str(fdb_root / "usr/local/lib"))
        dyld_path = env.get("DYLD_LIBRARY_PATH", "")
        fdb_lib = env["FDB_LIBRARY_PATH"]
        env["DYLD_LIBRARY_PATH"] = f"{fdb_lib}:{dyld_path}" if dyld_path else fdb_lib

    env.setdefault("FDB_STORAGE_ENGINE", "ssd")
    env.setdefault("FDBSERVER_EXTRA_ARGS", DEFAULT_FDB_KNOBS)
    env.setdefault("BRIDGE_FDB_RETRY_LIMIT", "20")
    env.setdefault("BRIDGE_FDB_RETRY_TIMEOUT_SECS", "120")
    return env


def _jsonrpc(url: str, method: str, params: list[Any] | None = None) -> Any:
    response = requests.post(
        url,
        json={"jsonrpc": "2.0", "method": method, "params": params or [], "id": 1},
        timeout=5,
    )
    response.raise_for_status()
    payload = response.json()
    if "error" in payload:
        raise RuntimeError(payload["error"])
    return payload["result"]


@dataclass
class ExternalBridgeHandle:
    """Running external bridge daemon."""

    process: subprocess.Popen[str]
    stop_file: Path
    rpc_url: str
    datadir: Path

    def wait_deposit_complete(self, drt_txid: str, timeout: int = 360) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            indices = _jsonrpc(self.rpc_url, "stratabridge_depositIndices")
            for deposit_idx in indices:
                info = _jsonrpc(self.rpc_url, "stratabridge_depositInfo", [deposit_idx])
                if info.get("deposit_request_txid") != drt_txid:
                    continue
                if info.get("status", {}).get("status") == "complete":
                    return info
            time.sleep(2)
        raise AssertionError(f"bridge deposit did not complete for DRT {drt_txid}")

    def stop(self) -> None:
        if self.process.poll() is not None:
            return
        self.stop_file.write_text("stop\n")
        try:
            self.process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=10)


def start_external_bridge(
    repo: Path,
    datadir: Path,
    bitcoind_props: dict[str, Any],
    genesis_l1_height: int,
    operator_count: int = DEFAULT_OPERATOR_COUNT,
    deposit_amount: int = 1_000_000_000,
) -> ExternalBridgeHandle:
    """Start bridge operators from an external checkout against the core Bitcoin node."""
    build_external_bridge(repo)

    bridge_run_dir = datadir / "external_bridge"
    bridge_run_dir.mkdir(parents=True, exist_ok=True)
    stop_file = bridge_run_dir / "stop"
    script_path = bridge_run_dir / "bridge_daemon.py"
    script_path.write_text(
        _daemon_script(
            bridge_run_dir,
            stop_file,
            bitcoind_props,
            genesis_l1_height,
            operator_count,
            deposit_amount,
        )
    )

    proc = subprocess.Popen(
        ["uv", "run", "python", str(script_path)],
        cwd=repo / "functional-tests",
        env=_bridge_process_env(repo),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    assert proc.stdout is not None

    deadline = time.monotonic() + 420
    captured: list[str] = []
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            output = "".join(captured)
            raise RuntimeError(f"external bridge exited during startup:\n{output}")

        readable, _, _ = select.select([proc.stdout], [], [], 1)
        if not readable:
            continue

        line = proc.stdout.readline()
        captured.append(line)
        if line.startswith(READY_PREFIX):
            info = json.loads(line.removeprefix(READY_PREFIX))
            return ExternalBridgeHandle(
                process=proc,
                stop_file=stop_file,
                rpc_url=info["rpc_url"],
                datadir=Path(info["datadir"]),
            )
        if line.startswith(STARTUP_LOG_PREFIXES):
            print(line, end="")
        if line.startswith(STAGE_PREFIX):
            info = json.loads(line.removeprefix(STAGE_PREFIX))
            if info.get("message") in ("ready", "stakes_confirmed"):
                rpc_url = info.get("rpc_url") or f"http://127.0.0.1:{DEFAULT_BRIDGE_RPC_PORT}"
                return ExternalBridgeHandle(
                    process=proc,
                    stop_file=stop_file,
                    rpc_url=rpc_url,
                    datadir=Path(info.get("datadir") or bridge_run_dir),
                )

    proc.terminate()
    output = "".join(captured)
    raise TimeoutError(f"timed out waiting for external bridge readiness:\n{output}")


def _daemon_script(
    run_dir: Path,
    stop_file: Path,
    bitcoind_props: dict[str, Any],
    genesis_l1_height: int,
    operator_count: int,
    deposit_amount: int,
) -> str:
    return f"""
import json
import logging
import sys
import time
from pathlib import Path

import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

sys.path.insert(0, str(Path.cwd()))

from constants import BRIDGE_NODE_DIR
from entry import generate_mtls_credentials
from envs.base_env import BaseEnv
from envs.btc_config import BitcoinEnvConfig
from envs.live_env import StrataLiveEnv
from factory.asm_rpc import AsmRpcFactory
from factory.bridge_operator import BridgeOperatorFactory
from factory.bridge_operator.config_cfg import BridgeConfigParams
from factory.bridge_operator.params_cfg import BridgeProtocolParams
from factory.fdb import FdbFactory
from factory.mosaic import MosaicFactory, MosaicFactoryConfig
from factory.s2 import S2Factory
from rpc.client import JsonrpcClient
from utils.mosaic import get_circuit_path, get_peer_configs
from utils.network import wait_until_p2p_connected
from utils.service_names import get_operator_service_name
from utils.utils import generate_blocks, wait_until, wait_until_bridge_ready

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s %(message)s")

BITCOIND_PROPS = {json.dumps(bitcoind_props)}
RUN_DIR = Path({json.dumps(str(run_dir))})
STOP_FILE = Path({json.dumps(str(stop_file))})
GENESIS_L1_HEIGHT = {genesis_l1_height}
OPERATOR_COUNT = {operator_count}
DEPOSIT_AMOUNT = {deposit_amount}


class ExternalBitcoinService:
    def __init__(self, props):
        self.props = props

    def create_rpc(self):
        return BitcoindClient(base_url=self.props["rpc_url"], network="regtest")

    def healthcheck(self):
        self.create_rpc().proxy.getblockchaininfo()

    def is_started(self):
        return True

    def check_status(self):
        self.healthcheck()
        return True

    def get_status_msg(self):
        return None

    def stop(self):
        return None


class ExternalBitcoinBridgeEnv(BaseEnv):
    def __init__(self):
        btc_config = BitcoinEnvConfig(auto_mine=True, initial_blocks=GENESIS_L1_HEIGHT)
        protocol = BridgeProtocolParams(deposit_amount=DEPOSIT_AMOUNT)
        super().__init__(
            OPERATOR_COUNT,
            bridge_protocol_params=protocol,
            bridge_config_params=BridgeConfigParams(),
            btc_config=btc_config,
        )
        self.initial_blocks = GENESIS_L1_HEIGHT

    def init(self, ectx):
        svcs = {{"bitcoin": ExternalBitcoinService(BITCOIND_PROPS)}}
        brpc = svcs["bitcoin"].create_rpc()
        wallet_addr = brpc.proxy.getnewaddress()
        miner = generate_blocks(brpc, self.btc_config.block_generation_interval_secs, wallet_addr)

        fdb = self.setup_fdb(ectx, "external-core")
        svcs["fdb"] = fdb

        mosaic_fac = ectx.get_factory("mosaic")
        mosaic_factory_config = MosaicFactoryConfig(
            circuit_path=get_circuit_path(),
            storage_cluster_file=fdb.props["cluster_file"],
            fdb_prefix=self.fdb_root_directory_prefix,
            all_peers=get_peer_configs(self.num_operators),
        )

        for idx in range(self.num_operators):
            mosaic_service = mosaic_fac.create_mosaic_service(idx, mosaic_factory_config)
            s2_service, bridge_node, asm_service = self.create_operator(
                ectx,
                idx,
                BITCOIND_PROPS,
                brpc,
                fdb.props,
                mosaic_service.props["rpc_url"],
            )
            self.fund_operator(brpc, bridge_node.props, wallet_addr)
            wait_until_bridge_ready(bridge_node.create_rpc())
            svcs[f"s2_{{idx}}"] = s2_service
            svcs[f"bridge_node_{{idx}}"] = bridge_node
            svcs["asm_rpc"] = asm_service
            svcs[f"mosaic_{{idx}}"] = mosaic_service

        return StrataLiveEnv(svcs, miner)


class KeepBridgeRunning(flexitest.Test):
    def __init__(self, ctx):
        ctx.set_env(ExternalBitcoinBridgeEnv())

    def main(self, ctx):
        bridge_nodes = [ctx.get_service(f"bridge_node_{{idx}}") for idx in range(OPERATOR_COUNT)]
        bridge_rpcs = [bridge_node.create_rpc() for bridge_node in bridge_nodes]

        bridge_stage("p2p_wait_start")
        wait_until_p2p_connected(bridge_rpcs)
        bridge_stage("p2p_connected")

        bitcoin_rpc = ctx.get_service("bitcoin").create_rpc()
        confirmed_stakes = wait_for_confirmed_stakes(bridge_rpcs[0], bitcoin_rpc)
        bridge_stage("stakes_confirmed", stakes=confirmed_stakes)

        rpc_url = "http://127.0.0.1:{DEFAULT_BRIDGE_RPC_PORT}"
        bridge_stage("ready", rpc_url=rpc_url, datadir=str(RUN_DIR))
        print(
            "BRIDGE_READY "
            + json.dumps({{"rpc_url": rpc_url, "datadir": str(RUN_DIR)}}),
            flush=True,
        )
        while not STOP_FILE.exists():
            time.sleep(1)
        return True


def bridge_stage(message, **fields):
    payload = {{"message": message}}
    payload.update(fields)
    print("BRIDGE_STAGE " + json.dumps(payload), flush=True)


def wait_for_confirmed_stakes(bridge_rpc, bitcoin_rpc):
    deadline = time.monotonic() + 300
    last_report = 0
    last_error = None
    last_stakes = None

    while time.monotonic() < deadline:
        try:
            stakes = bridge_rpc.stratabridge_stakeStatus()
            last_stakes = stakes
            now = time.monotonic()
            if now - last_report >= 5:
                bridge_stage("stake_status", stakes=stakes)
                last_report = now

            if len(stakes) < OPERATOR_COUNT:
                time.sleep(1)
                continue

            ready = True
            for stake in stakes:
                state = stake.get("state")
                if state not in ("confirmed", "preimage_revealed", "unstaked"):
                    ready = False
                    break

                stake_txid = stake.get("stake_txid")
                if state == "confirmed":
                    if stake_txid is None:
                        ready = False
                        break
                    tx_info = bitcoin_rpc.proxy.getrawtransaction(stake_txid, True)
                    if "blockhash" not in tx_info:
                        ready = False
                        break

            if ready:
                return stakes
        except Exception as exc:
            last_error = type(exc).__name__ + ": " + str(exc)
            now = time.monotonic()
            if now - last_report >= 5:
                bridge_stage("stake_status_error", error=last_error)
                last_report = now

        time.sleep(1)

    raise TimeoutError(
        "bridge operators did not reach confirmed stake status; "
        + "last_error="
        + repr(last_error)
        + "; last_stakes="
        + repr(last_stakes)
    )


runtime = flexitest.TestRuntime(
    {{}},
    str(RUN_DIR / "_dd"),
    {{
        "fdb": FdbFactory([12700 + i for i in range(100)]),
        "s2": S2Factory([12800 + i for i in range(100)]),
        "bofac": BridgeOperatorFactory([13000 + i for i in range(100)]),
        "asm_rpc": AsmRpcFactory([12600 + i for i in range(100)]),
        "mosaic": MosaicFactory([12900 + i for i in range(100)]),
    }},
)

gen_s2_tls_script_path = str((Path.cwd().parent / "docker" / "gen_s2_tls.sh").resolve())
for operator_idx in range(OPERATOR_COUNT):
    generate_mtls_credentials(gen_s2_tls_script_path, str(RUN_DIR / "_dd"), operator_idx)

runtime.prepare_test("keep_bridge_running", KeepBridgeRunning)
results = runtime.run_tests(["keep_bridge_running"])
if results["keep_bridge_running"]["status"] != "OK":
    print("BRIDGE_ERROR " + json.dumps(results), flush=True)
    raise SystemExit(json.dumps(results, indent=2))
"""
