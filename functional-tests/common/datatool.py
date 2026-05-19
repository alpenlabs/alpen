import json
import os
import shutil
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

from common.config.config import BitcoindConfig

DEFAULT_OL_BLOCK_TIME_MS = 5_000


def _env_truthy(name: str) -> bool:
    return os.getenv(name, "").lower() in ("1", "true", "yes", "on")


def _checkpoint_predicate() -> str:
    if predicate := os.getenv("ALPEN_CHECKPOINT_PREDICATE_MODE"):
        return predicate
    if _env_truthy("ALPEN_USE_LOCAL_SP1_PREDICATES"):
        return "sp1-groth16"
    return "bip340-schnorr-test"


def _alpen_predicate() -> str:
    if predicate := os.getenv("ALPEN_ALPEN_PREDICATE_MODE"):
        return predicate
    if _env_truthy("ALPEN_USE_LOCAL_SP1_PREDICATES"):
        return "sp1-groth16"
    return "bip340-schnorr-test"


def _uses_local_sp1_predicates() -> bool:
    return (
        _env_truthy("ALPEN_USE_LOCAL_SP1_PREDICATES")
        or os.getenv("ALPEN_CHECKPOINT_PREDICATE_MODE") == "sp1-groth16"
        or os.getenv("ALPEN_ALPEN_PREDICATE_MODE") == "sp1-groth16"
    )


def _sp1_elf_dir() -> Path | None:
    if not _uses_local_sp1_predicates():
        return None
    if configured_dir := os.getenv("ALPEN_SP1_ELF_DIR"):
        return Path(configured_dir)
    return Path(__file__).resolve().parents[1] / "_sp1_elfs" / "local"


def run_datatool(
    args: list[str], bconfig: BitcoindConfig | None = None
) -> subprocess.CompletedProcess[str]:
    """Runs strata-datatool with optional Bitcoin RPC credentials."""
    tool = shutil.which("strata-datatool")
    if tool is None:
        raise RuntimeError("strata-datatool not found on PATH")

    cmd = [tool, "-b", "regtest"]
    if bconfig is not None:
        cmd.extend(
            [
                "--bitcoin-rpc-url",
                bconfig.rpc_url,
                "--bitcoin-rpc-user",
                bconfig.rpc_user,
                "--bitcoin-rpc-password",
                bconfig.rpc_password,
            ]
        )
    cmd.extend(args)

    res = subprocess.run(cmd, capture_output=True, text=True)
    if res.returncode != 0:
        details = res.stderr.strip() or res.stdout.strip()
        raise RuntimeError(f"strata-datatool {args[0]} failed: {details}")
    return res


def run_datatool_for_file(
    args: list[str],
    output_path: Path,
    bconfig: BitcoindConfig | None = None,
    attempts: int = 60,
    sleep_secs: float = 1.0,
) -> subprocess.CompletedProcess[str]:
    """Runs strata-datatool until it creates the expected output file."""
    last_details = ""
    for attempt in range(1, attempts + 1):
        try:
            res = run_datatool(args, bconfig)
        except RuntimeError as err:
            last_details = str(err)
        else:
            if output_path.exists() and output_path.stat().st_size > 0:
                return res
            last_details = (res.stderr.strip() or res.stdout.strip()).strip()

        if attempt < attempts:
            time.sleep(sleep_secs)

    details = (": " + last_details) if last_details else ""
    raise RuntimeError(
        "strata-datatool " + args[0] + " did not create " + str(output_path) + details
    )


def _patch_first_key(node, key: str, value: str) -> bool:
    if isinstance(node, dict):
        if key in node:
            node[key] = value
            return True
        return any(_patch_first_key(v, key, value) for v in node.values())
    if isinstance(node, list):
        return any(_patch_first_key(v, key, value) for v in node)
    return False


def _patch_all_alpen_account_predicates(node, value: str) -> bool:
    patched = False
    if isinstance(node, dict):
        accounts = node.get("accounts")
        if isinstance(accounts, dict):
            alpen_account = accounts.get("01" * 32)
            if isinstance(alpen_account, dict) and "predicate" in alpen_account:
                alpen_account["predicate"] = value
                patched = True
        if "account_id" in node and "predicate" in node:
            account_id = str(node["account_id"]).lower().removeprefix("0x")
            if account_id == "01" * 32:
                node["predicate"] = value
                patched = True
        for child in node.values():
            patched = _patch_all_alpen_account_predicates(child, value) or patched
    elif isinstance(node, list):
        for child in node:
            patched = _patch_all_alpen_account_predicates(child, value) or patched
    return patched


def _patch_json(path: Path, patcher) -> None:
    data = json.loads(path.read_text())
    if not patcher(data):
        raise RuntimeError(f"failed to patch generated params: {path}")
    path.write_text(json.dumps(data, indent=2) + "\n")


def patch_generated_predicates(params_path: Path, *, kind: str) -> None:
    if kind in ("rollup", "asm"):
        if predicate := os.getenv("ALPEN_CHECKPOINT_PREDICATE_FULL"):
            _patch_json(
                params_path,
                lambda data: _patch_first_key(data, "checkpoint_predicate", predicate),
            )
    if kind == "ol":
        if predicate := os.getenv("ALPEN_ALPEN_PREDICATE_FULL"):
            _patch_json(
                params_path,
                lambda data: _patch_all_alpen_account_predicates(data, predicate),
            )


def ensure_priv_key(path: Path) -> None:
    """Checks if the path already exists and generates private key if not"""
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


def get_operator_xprivs(datadir, operator_fname) -> list[str]:
    if configured_operator_keys := os.getenv("ALPEN_BRIDGE_OPERATOR_XPRIVS_JSON"):
        operator_xprivs = json.loads(configured_operator_keys)
        if not isinstance(operator_xprivs, list) or not operator_xprivs:
            raise RuntimeError("ALPEN_BRIDGE_OPERATOR_XPRIVS_JSON must be a non-empty JSON list")
        operator_key_path = datadir / operator_fname
        operator_key_path.write_text("\n".join(operator_xprivs) + "\n")
        return operator_xprivs

    # Generate operator keys
    operator_key_path = datadir / operator_fname
    ensure_priv_key(operator_key_path)
    operator_xpriv = operator_key_path.read_text().strip()
    operator_xprivs = [operator_xpriv]
    return operator_xprivs


@dataclass
class RollupParamsArtifacts:
    params_path: Path
    sequencer_key_path: Path | None
    sequencer_pubkey: str | None
    operator_keys: list[str]


def write_sequencer_runtime_config(
    config_path: Path,
    ol_block_time_ms: int = DEFAULT_OL_BLOCK_TIME_MS,
) -> Path:
    """Writes the sequencer runtime config TOML."""
    config_path.write_text(
        "\n".join(
            [
                "[sequencer]",
                f"ol_block_time_ms = {ol_block_time_ms}",
                "",
            ]
        )
    )
    return config_path


def generate_rollup_params_unchecked(
    datadir: Path,
    bconfig: BitcoindConfig,
    genesis_l1_height: int,
    seq_fname: str = "sequencer_root_key",
) -> RollupParamsArtifacts:
    """Generates rollup params with ``CredRule::Unchecked``.

    A sequencer key is generated for the signer to load (so it can fulfill
    block signing duties), but the key is NOT embedded in the rollup params,
    keeping the cred rule as ``Unchecked``.  ``SignRevealTx`` duties are
    handled in-process and never reach the signer.
    """
    sequencer_key_path = datadir / seq_fname
    ensure_priv_key(sequencer_key_path)
    operator_xprivs = get_operator_xprivs(datadir, "bridge-operator_keys")
    params_path = datadir / "rollup-params.json"

    args = [
        "genparams",
        "--checkpoint-predicate",
        _checkpoint_predicate(),
        "--name",
        "ALPN",
        "--genesis-l1-height",
        str(genesis_l1_height),
        "-o",
        str(params_path),
    ]
    if elf_dir := _sp1_elf_dir():
        args.extend(["--elf-dir", str(elf_dir)])

    for opkey in operator_xprivs:
        args.extend(["--opkey", opkey])

    run_datatool_for_file(args, params_path, bconfig)
    patch_generated_predicates(params_path, kind="rollup")
    return RollupParamsArtifacts(
        params_path=params_path,
        sequencer_key_path=sequencer_key_path,
        sequencer_pubkey=None,
        operator_keys=operator_xprivs,
    )


def generate_rollup_params(
    datadir: Path,
    bconfig: BitcoindConfig,
    genesis_l1_height: int,
    seq_fname="sequencer_root_key",
) -> RollupParamsArtifacts:
    # Generate sequencer keys
    sequencer_key_path = datadir / seq_fname
    ensure_priv_key(sequencer_key_path)
    sequencer_pubkey = generate_sequencer_pubkey(sequencer_key_path)
    operator_xprivs = get_operator_xprivs(datadir, "bridge-operator_keys")

    params_path = datadir / "rollup-params.json"

    args = [
        "genparams",
        "--checkpoint-predicate",
        _checkpoint_predicate(),
        "--name",
        "ALPN",
        "--genesis-l1-height",
        str(genesis_l1_height),
        "--seqkey",
        sequencer_pubkey,
        "-o",
        str(params_path),
    ]
    if elf_dir := _sp1_elf_dir():
        args.extend(["--elf-dir", str(elf_dir)])

    for opkey in operator_xprivs:
        args.extend(["--opkey", opkey])

    run_datatool_for_file(args, params_path, bconfig)
    patch_generated_predicates(params_path, kind="rollup")
    return RollupParamsArtifacts(params_path, sequencer_key_path, sequencer_pubkey, operator_xprivs)


def generate_sequencer_pubkey(sequencer_key_path: Path) -> str:
    res = run_datatool(["genseqpubkey", "-f", str(sequencer_key_path)])
    sequencer_pubkey = res.stdout.strip()
    if not sequencer_pubkey:
        raise RuntimeError("strata-datatool genseqpubkey returned empty output")
    return sequencer_pubkey


def generate_ol_params(
    datadir: Path,
    bconfig: BitcoindConfig,
    genesis_l1_height: int,
    alpen_chain_config: Path | str | None = None,
) -> Path:
    """Generates OL params via ``strata-datatool gen-ol-params``."""
    params_path = datadir / "ol-params.json"

    args = [
        "gen-ol-params",
        "--alpen-predicate",
        _alpen_predicate(),
        "--genesis-l1-height",
        str(genesis_l1_height),
        "-o",
        str(params_path),
    ]
    if alpen_chain_config is not None:
        args.extend(["--alpen-chain-config", str(alpen_chain_config)])

    run_datatool_for_file(args, params_path, bconfig)
    patch_generated_predicates(params_path, kind="ol")
    return params_path


def generate_asm_params(
    datadir: Path,
    bconfig: BitcoindConfig,
    genesis_l1_height: int,
    operator_xprivs: list[str],
    ol_params_path: Path | None = None,
    sequencer_pubkey: str | None = None,
    admin_confirmation_depth: int | None = None,
) -> Path:
    params_path = datadir / "asm-params.json"

    args = [
        "gen-asm-params",
        "--checkpoint-predicate",
        _checkpoint_predicate(),
        "--name",
        "ALPN",
        "--genesis-l1-height",
        str(genesis_l1_height),
        "-o",
        str(params_path),
    ]
    if ol_params_path is not None:
        args.extend(["--ol-params", str(ol_params_path)])
    if sequencer_pubkey is not None:
        args.extend(["--seqkey", sequencer_pubkey])
    if admin_confirmation_depth is not None:
        args.extend(["--confirmation-depth", str(admin_confirmation_depth)])
    for opkey in operator_xprivs:
        args.extend(["--opkey", opkey])

    run_datatool_for_file(args, params_path, bconfig)
    patch_generated_predicates(params_path, kind="asm")
    return params_path
