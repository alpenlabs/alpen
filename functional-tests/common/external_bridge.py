"""Helpers for running an external strata-bridge checkout in functional tests."""

import hashlib
import hmac
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
from coincurve import PublicKey

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
SECP256K1_ORDER = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
TESTNET_XPRV_VERSION = bytes.fromhex("04358394")
HARDENED_OFFSET = 0x80000000


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

    raise RuntimeError("External strata-bridge checkout not found. Set ALPEN_EXTERNAL_BRIDGE_REPO.")


def load_bridge_operator_pubkeys(repo: Path, count: int = DEFAULT_OPERATOR_COUNT) -> list[str]:
    """Load MuSig2 operator public keys from the external bridge test artifacts."""
    keys_path = repo / "functional-tests/artifacts/keys.json"
    keys = json.loads(keys_path.read_text())
    if len(keys) < count:
        raise RuntimeError(f"requested {count} bridge operators, only {len(keys)} keys available")
    return [entry["MUSIG2_KEY"] for entry in keys[:count]]


def load_bridge_operator_musig_xprivs(repo: Path, count: int = DEFAULT_OPERATOR_COUNT) -> list[str]:
    """Derive xprivs whose direct pubkeys match external bridge MuSig2 keys."""
    keys_path = repo / "functional-tests/artifacts/keys.json"
    keys = json.loads(keys_path.read_text())
    if len(keys) < count:
        raise RuntimeError(f"requested {count} bridge operators, only {len(keys)} keys available")

    derived = []
    for entry in keys[:count]:
        secret = _derive_bridge_musig_secret(bytes.fromhex(entry["SEED"]))
        xpriv = _xpriv_from_secret(secret)
        if _xonly_from_secret(secret) != entry["MUSIG2_KEY"]:
            raise RuntimeError("derived bridge operator xpriv does not match MuSig2 artifact")
        derived.append(xpriv)
    return derived


def compute_bridge_aggregate_pubkey(operator_pubkeys: list[str]) -> str:
    """Compute the MuSig2 aggregate key used by `alpen deposit`."""
    pubkeys = [_parse_even_xonly_pubkey(pubkey) for pubkey in operator_pubkeys]
    serialized_pubkeys = [pubkey.format(compressed=True) for pubkey in pubkeys]

    second_unique_key = next(
        (pubkey for pubkey in serialized_pubkeys[1:] if pubkey != serialized_pubkeys[0]),
        None,
    )
    pubkey_list_hash = _tagged_hash(b"KeyAgg list", b"".join(serialized_pubkeys))

    effective_pubkeys = []
    for pubkey, serialized_pubkey in zip(pubkeys, serialized_pubkeys, strict=True):
        if second_unique_key is not None and serialized_pubkey == second_unique_key:
            coefficient = 1
        else:
            coefficient = (
                int.from_bytes(
                    _tagged_hash(
                        b"KeyAgg coefficient",
                        pubkey_list_hash + serialized_pubkey,
                    ),
                    "big",
                )
                % SECP256K1_ORDER
            )

        effective_pubkeys.append(pubkey.multiply(coefficient.to_bytes(32, "big")))

    return PublicKey.combine_keys(effective_pubkeys).format(compressed=True)[1:].hex()


def _parse_even_xonly_pubkey(pubkey: str) -> PublicKey:
    """Parse bridge artifact MuSig2 keys as even compressed secp256k1 pubkeys."""
    key_hex = pubkey.removeprefix("0x")
    if len(key_hex) == 64:
        key_hex = "02" + key_hex
    if len(key_hex) != 66 or not key_hex.startswith("02"):
        raise ValueError(f"expected even x-only/compressed pubkey, got {pubkey}")
    return PublicKey(bytes.fromhex(key_hex))


def _tagged_hash(tag: bytes, payload: bytes) -> bytes:
    tag_hash = hashlib.sha256(tag).digest()
    return hashlib.sha256(tag_hash + tag_hash + payload).digest()


def _derive_bridge_musig_secret(seed: bytes) -> bytes:
    master_secret, chain_code = _bip32_master_key(seed)
    secret = master_secret
    for index in (20_000, 20, 20, 101):
        secret, chain_code = _bip32_hardened_child(secret, chain_code, index)
    return _make_even_secret(secret)


def _bip32_master_key(seed: bytes) -> tuple[bytes, bytes]:
    digest = hmac.new(b"Bitcoin seed", seed, hashlib.sha512).digest()
    return _valid_secret(digest[:32]), digest[32:]


def _bip32_hardened_child(secret: bytes, chain_code: bytes, index: int) -> tuple[bytes, bytes]:
    data = b"\x00" + secret + (index + HARDENED_OFFSET).to_bytes(4, "big")
    digest = hmac.new(chain_code, data, hashlib.sha512).digest()
    child_int = (
        int.from_bytes(digest[:32], "big") + int.from_bytes(secret, "big")
    ) % SECP256K1_ORDER
    return _valid_secret(child_int.to_bytes(32, "big")), digest[32:]


def _make_even_secret(secret: bytes) -> bytes:
    if PublicKey.from_valid_secret(secret).format(compressed=True)[0] == 0x03:
        secret_int = SECP256K1_ORDER - int.from_bytes(secret, "big")
        return secret_int.to_bytes(32, "big")
    return secret


def _xonly_from_secret(secret: bytes) -> str:
    return PublicKey.from_valid_secret(secret).format(compressed=True)[1:].hex()


def _xpriv_from_secret(secret: bytes) -> str:
    payload = (
        TESTNET_XPRV_VERSION
        + b"\x00"
        + b"\x00\x00\x00\x00"
        + b"\x00\x00\x00\x00"
        + bytes(32)
        + b"\x00"
        + secret
    )
    checksum = hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4]
    return _base58_encode(payload + checksum)


def _valid_secret(secret: bytes) -> bytes:
    value = int.from_bytes(secret, "big")
    if value == 0 or value >= SECP256K1_ORDER:
        raise ValueError("invalid BIP32 secret")
    return secret


def _base58_encode(data: bytes) -> str:
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    value = int.from_bytes(data, "big")
    encoded = ""
    while value:
        value, remainder = divmod(value, 58)
        encoded = alphabet[remainder] + encoded
    leading_zeroes = len(data) - len(data.lstrip(b"\x00"))
    return "1" * leading_zeroes + encoded


def _extract_git_rev(cargo_toml: str, package: str) -> str:
    ref_arg, ref = _extract_git_ref(cargo_toml, package)
    if ref_arg != "--rev":
        raise RuntimeError(f"expected {package} to use a git rev, got {ref_arg} {ref}")
    return ref


def _extract_git_ref(cargo_toml: str, package: str) -> tuple[str, str]:
    pattern = rf"{re.escape(package)}[^\n]*(rev|tag) = \"([^\"]+)\""
    match = re.search(pattern, cargo_toml)
    if match is None:
        raise RuntimeError(f"failed to extract {package} git ref from Cargo.toml")
    ref_kind, ref = match.groups()
    return f"--{ref_kind}", ref


def _core_repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


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
    core_cargo_toml = (_core_repo_root() / "Cargo.toml").read_text()
    asm_ref_arg, asm_ref = _extract_git_ref(core_cargo_toml, "strata-asm-worker")

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

    asm_src = _checkout_asm_repo(install_root, asm_ref_arg, asm_ref)
    subprocess.run(
        ["cargo", "build", "-p", "strata-asm-sp1-guest-builder", "--release"],
        cwd=asm_src,
        env=env,
        check=True,
    )

    asm_runner_ref_stamp = install_root / ".strata-asm-runner-ref"
    asm_runner_ref = f"sp1 {asm_ref_arg} {asm_ref}\n"
    asm_runner_needs_install = shutil.which("strata-asm-runner", path=str(bin_dir)) is None
    if not asm_runner_needs_install:
        asm_runner_needs_install = (
            not asm_runner_ref_stamp.exists() or asm_runner_ref_stamp.read_text() != asm_runner_ref
        )

    if asm_runner_needs_install:
        subprocess.run(
            [
                "cargo",
                "install",
                "--locked",
                "--force",
                "--root",
                str(install_root),
                "--path",
                str(asm_src / "bin/asm-runner"),
                "--features",
                "sp1",
            ],
            check=True,
        )
        asm_runner_ref_stamp.write_text(asm_runner_ref)


def _checkout_asm_repo(install_root: Path, ref_arg: str, ref: str) -> Path:
    """Checkout the ASM repo ref used by core so SP1 runner ELFs match it."""
    asm_src = install_root / "src/asm"
    if not (asm_src / ".git").exists():
        asm_src.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["git", "clone", "https://github.com/alpenlabs/asm", str(asm_src)],
            check=True,
        )

    subprocess.run(["git", "fetch", "--tags", "origin"], cwd=asm_src, check=True)
    checkout_ref = ref if ref_arg == "--rev" else f"tags/{ref}"
    subprocess.run(["git", "checkout", "--detach", checkout_ref], cwd=asm_src, check=True)
    subprocess.run(["git", "clean", "-ffd"], cwd=asm_src, check=True)
    return asm_src


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
    asm_elf_dir = ft_root / "_dd/.bin/src/asm/guest-builder/sp1/elfs"
    env.setdefault("ASM_PROVER_BACKEND", "sp1")
    env.setdefault("ALPEN_EXTERNAL_BRIDGE_ASM_ELF_PATH", str(asm_elf_dir / "asm.elf"))
    env.setdefault("ALPEN_EXTERNAL_BRIDGE_MOHO_ELF_PATH", str(asm_elf_dir / "moho.elf"))
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

    def rpc(self, method: str, params: list[Any] | None = None) -> Any:
        return _jsonrpc(self.rpc_url, method, params)

    def wait_deposit_complete(self, drt_txid: str, timeout: int = 360) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            indices = self.rpc("stratabridge_depositIndices")
            for deposit_idx in indices:
                info = self.rpc("stratabridge_depositInfo", [deposit_idx])
                if info.get("deposit_request_txid") != drt_txid:
                    continue
                if info.get("status", {}).get("status") == "complete":
                    return info
            time.sleep(2)
        raise AssertionError(f"bridge deposit did not complete for DRT {drt_txid}")

    def wait_pending_withdrawal_seen(self, timeout: int = 420) -> tuple[int, dict[str, Any]]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pending = self.rpc("stratabridge_pendingWithdrawals")
            if pending:
                deposit_idx = int(pending[0])
                info = self.rpc("stratabridge_pendingWithdrawalInfo", [deposit_idx])
                if info is not None:
                    return deposit_idx, info
            time.sleep(2)
        raise AssertionError("bridge did not observe a pending withdrawal")

    def wait_pending_withdrawals_clear(self, timeout: int = 600) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if not self.rpc("stratabridge_pendingWithdrawals"):
                return
            time.sleep(2)
        raise AssertionError("bridge pending withdrawals did not clear")

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
    asm_params_path: Path | None = None,
) -> ExternalBridgeHandle:
    """Start bridge operators from an external checkout against the core Bitcoin node."""
    build_external_bridge(repo)
    asm_params = json.loads(Path(asm_params_path).read_text()) if asm_params_path else None

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
            asm_params,
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

    deadline = time.monotonic() + int(
        os.environ.get("ALPEN_EXTERNAL_BRIDGE_STARTUP_TIMEOUT", "2400")
    )
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
    asm_params: dict[str, Any] | None,
) -> str:
    return f"""
import json
import logging
import os
import resource
import sys
import time
from dataclasses import dataclass
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
from factory.asm_rpc.config_cfg import Duration
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


def _raise_nofile_limit():
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    target = int(os.environ.get("ALPEN_EXTERNAL_BRIDGE_NOFILE", "8192"))
    if hard != resource.RLIM_INFINITY:
        target = min(target, hard)
    if soft < target:
        resource.setrlimit(resource.RLIMIT_NOFILE, (target, hard))


try:
    _raise_nofile_limit()
except Exception as exc:
    logging.warning("failed to raise bridge nofile limit: %s", exc)


BITCOIND_PROPS = {json.dumps(bitcoind_props)}
RUN_DIR = Path({json.dumps(str(run_dir))})
STOP_FILE = Path({json.dumps(str(stop_file))})
GENESIS_L1_HEIGHT = {genesis_l1_height}
OPERATOR_COUNT = {operator_count}
DEPOSIT_AMOUNT = {deposit_amount}
ASM_PARAMS = {json.dumps(asm_params)}


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


@dataclass
class Sp1BackendConfig:
    asm_elf_path: str
    moho_elf_path: str
    kind: str = "sp1"


@dataclass
class Sp1OrchestratorConfig:
    tick_interval: Duration
    max_concurrent_proofs: int
    proof_db_path: str
    backend: Sp1BackendConfig


class ExternalBitcoinBridgeEnv(BaseEnv):
    def __init__(self):
        btc_config = BitcoinEnvConfig(auto_mine=True, initial_blocks=GENESIS_L1_HEIGHT)
        bridge_subprotocol = _bridge_subprotocol()
        protocol_kwargs = {{"deposit_amount": DEPOSIT_AMOUNT}}
        config_kwargs = {{}}
        if bridge_subprotocol is not None:
            protocol_kwargs.update(
                magic_bytes=ASM_PARAMS["magic"],
                deposit_amount=int(bridge_subprotocol["denomination"]),
                operator_fee=int(bridge_subprotocol["operator_fee"]),
                recovery_delay=int(bridge_subprotocol["recovery_delay"]),
            )
            assignment_duration = int(bridge_subprotocol["assignment_duration"])
            config_kwargs["min_withdrawal_fulfillment_window"] = int(
                os.environ.get(
                    "ALPEN_EXTERNAL_BRIDGE_MIN_WITHDRAWAL_FULFILLMENT_WINDOW",
                    str(max(0, min(16, assignment_duration // 4))),
                )
            )
        protocol = BridgeProtocolParams(**protocol_kwargs)
        bridge_config = BridgeConfigParams(**config_kwargs)
        super().__init__(
            OPERATOR_COUNT,
            bridge_protocol_params=protocol,
            bridge_config_params=bridge_config,
            btc_config=btc_config,
        )
        self.initial_blocks = GENESIS_L1_HEIGHT

    def _build_orchestrator_config(self, ectx):
        envdd_path = Path(ectx.envdd_path)
        proof_db_path = str((envdd_path / "asm_rpc" / "proof_db").resolve())
        asm_elf_path = os.environ.get("ALPEN_EXTERNAL_BRIDGE_ASM_ELF_PATH")
        moho_elf_path = os.environ.get("ALPEN_EXTERNAL_BRIDGE_MOHO_ELF_PATH")
        if not asm_elf_path or not moho_elf_path:
            raise RuntimeError("missing SP1 ASM/Moho ELF paths for external bridge ASM runner")
        return Sp1OrchestratorConfig(
            tick_interval=Duration(secs=1, nanos=0),
            max_concurrent_proofs=4,
            proof_db_path=proof_db_path,
            backend=Sp1BackendConfig(
                asm_elf_path=asm_elf_path,
                moho_elf_path=moho_elf_path,
            ),
        )

    def _ensure_rollup_params(self, ectx, bitcoind_rpc):
        if ASM_PARAMS is None:
            return super()._ensure_rollup_params(ectx, bitcoind_rpc)
        if self._bridge_genesis_height is not None and self._rollup_params_path is not None:
            return

        self._bridge_genesis_height = int(ASM_PARAMS["anchor"]["block"]["height"])
        envdd_path = Path(ectx.envdd_path)
        asm_params_path = envdd_path / "generated" / "asm-params.json"
        asm_params_path.parent.mkdir(parents=True, exist_ok=True)
        asm_params_path.write_text(json.dumps(_asm_params_for_runner(), indent=4) + "\\n")
        self._rollup_params_path = str(asm_params_path)

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
            bridge_stage("operator_start", operator_idx=idx)
            mosaic_service = mosaic_fac.create_mosaic_service(idx, mosaic_factory_config)
            bridge_stage("mosaic_ready", operator_idx=idx)
            s2_service, bridge_node, asm_service = self.create_operator(
                ectx,
                idx,
                BITCOIND_PROPS,
                brpc,
                fdb.props,
                mosaic_service.props["rpc_url"],
            )
            bridge_stage("bridge_node_started", operator_idx=idx)
            self.fund_operator(brpc, bridge_node.props, wallet_addr)
            bridge_stage("operator_funded", operator_idx=idx)
            wait_until_bridge_ready(bridge_node.create_rpc())
            bridge_stage("bridge_node_ready", operator_idx=idx)
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

        for idx, bridge_node in enumerate(bridge_nodes):
            bridge_stage("bridge_bootstrap_wait_start", operator_idx=idx)
            wait_for_bridge_bootstrapped(bridge_node.props["logfile"], idx)

        bitcoin_rpc = ctx.get_service("bitcoin").create_rpc()
        confirmed_stakes = wait_for_confirmed_stakes(bridge_rpcs[0], bitcoin_rpc)
        bridge_stage("stakes_confirmed", stakes=confirmed_stakes)
        ctx.env.stop_miner()
        bridge_stage("bridge_auto_miner_stopped")

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


def _bridge_subprotocol():
    if ASM_PARAMS is None:
        return None
    for subprotocol in ASM_PARAMS.get("subprotocols", []):
        if "Bridge" in subprotocol:
            return subprotocol["Bridge"]
    raise RuntimeError("Bridge subprotocol not found in ASM params")


def _asm_params_for_runner():
    params = json.loads(json.dumps(ASM_PARAMS))
    if os.environ.get("ALPEN_EXTERNAL_BRIDGE_LEGACY_ASM_PARAMS", "") not in (
        "1",
        "true",
        "yes",
        "on",
    ):
        return params
    for subprotocol in params.get("subprotocols", []):
        admin = subprotocol.get("Admin")
        if admin is None:
            continue
        depths = admin.pop("confirmation_depths", None)
        if depths is not None and "confirmation_depth" not in admin:
            admin["confirmation_depth"] = int(
                depths.get("operator_update", next(iter(depths.values())))
            )
    return params


def bridge_stage(message, **fields):
    payload = {{"message": message}}
    payload.update(fields)
    print("BRIDGE_STAGE " + json.dumps(payload), flush=True)


def wait_for_bridge_bootstrapped(logfile, operator_idx):
    deadline = time.monotonic() + int(
        os.environ.get("ALPEN_EXTERNAL_BRIDGE_BOOTSTRAP_TIMEOUT", "1800")
    )
    path = Path(logfile)
    markers = (
        "orchestrator pipeline started",
        "node bootstrapping complete",
    )
    last_report = 0
    last_size = -1

    while time.monotonic() < deadline:
        if path.exists():
            text = path.read_text(encoding="utf-8", errors="ignore")
            if any(marker in text for marker in markers):
                bridge_stage("bridge_bootstrapped", operator_idx=operator_idx)
                return

            size = path.stat().st_size
            now = time.monotonic()
            if size != last_size and now - last_report >= 10:
                bridge_stage(
                    "bridge_bootstrap_waiting", operator_idx=operator_idx, log_bytes=size
                )
                last_report = now
                last_size = size

        time.sleep(2)

    raise TimeoutError("bridge operator did not finish bootstrap; logfile=" + str(path))


def wait_for_confirmed_stakes(bridge_rpc, bitcoin_rpc):
    deadline = time.monotonic() + int(os.environ.get("ALPEN_EXTERNAL_BRIDGE_STAKE_TIMEOUT", "1200"))
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
    os._exit(1)
"""
