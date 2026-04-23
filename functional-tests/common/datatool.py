import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

from common.config.config import BitcoindConfig

DEFAULT_OL_BLOCK_TIME_MS = 5_000


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
        "always-accept",
        "--name",
        "ALPN",
        "--genesis-l1-height",
        str(genesis_l1_height),
        "-o",
        str(params_path),
    ]
    for opkey in operator_xprivs:
        args.extend(["--opkey", opkey])

    run_datatool(args, bconfig)
    return RollupParamsArtifacts(
        params_path=params_path,
        sequencer_key_path=sequencer_key_path,
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
        "always-accept",
        "--name",
        "ALPN",
        "--genesis-l1-height",
        str(genesis_l1_height),
        "--seqkey",
        sequencer_pubkey,
        "-o",
        str(params_path),
    ]
    for opkey in operator_xprivs:
        args.extend(["--opkey", opkey])

    run_datatool(args, bconfig)
    return RollupParamsArtifacts(params_path, sequencer_key_path, operator_xprivs)


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
) -> Path:
    """Generates OL params via ``strata-datatool gen-ol-params``."""
    params_path = datadir / "ol-params.json"

    args = [
        "gen-ol-params",
        "--genesis-l1-height",
        str(genesis_l1_height),
        "-o",
        str(params_path),
    ]

    run_datatool(args, bconfig)
    return params_path


def generate_asm_params(
    datadir: Path,
    bconfig: BitcoindConfig,
    genesis_l1_height: int,
    operator_xprivs: list[str],
    ol_params_path: Path | None = None,
    admin_confirmation_depth: int | None = None,
) -> Path:
    params_path = datadir / "asm-params.json"

    args = [
        "gen-asm-params",
        "--checkpoint-predicate",
        "always-accept",
        "--name",
        "ALPN",
        "--genesis-l1-height",
        str(genesis_l1_height),
        "-o",
        str(params_path),
    ]
    if ol_params_path is not None:
        args.extend(["--ol-params", str(ol_params_path)])
    if admin_confirmation_depth is not None:
        args.extend(["--confirmation-depth", str(admin_confirmation_depth)])
    for opkey in operator_xprivs:
        args.extend(["--opkey", opkey])

    run_datatool(args, bconfig)
    return params_path
