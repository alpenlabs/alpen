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


def get_operator_pubkeys(datadir, operator_fname) -> list[str]:
    """Generates an operator xpriv and returns the derived compressed public keys."""
    operator_key_path = datadir / operator_fname
    ensure_priv_key(operator_key_path)
    res = run_datatool(["genoppubkey", "-f", str(operator_key_path)])
    pubkey = res.stdout.strip()
    if not pubkey:
        raise RuntimeError("strata-datatool genoppubkey returned empty output")
    return [pubkey]


def p2tr_bosd_from_compressed_pubkey(pubkey: str) -> str:
    """Builds a P2TR BOSD descriptor from a compressed public key."""
    pubkey = pubkey.strip().lower()
    if len(pubkey) != 66 or pubkey[:2] not in ("02", "03"):
        raise ValueError(f"invalid compressed public key: {pubkey}")

    xonly_pubkey = pubkey[2:]
    bytes.fromhex(xonly_pubkey)
    return f"04{xonly_pubkey}"


@dataclass
class SequencerArtifacts:
    """Sequencer key material and operator pubkeys consumed when building ASM params."""

    sequencer_key_path: Path
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


def generate_sequencer_artifacts(
    datadir: Path,
    use_unchecked_cred_rule: bool,
    seq_fname: str = "sequencer_root_key",
) -> SequencerArtifacts:
    """Ensures the sequencer key and operator pubkeys used to build ASM params.

    A sequencer key is always generated so the signer can fulfill block-signing
    duties. When ``use_unchecked_cred_rule`` is True, the sequencer pubkey is
    NOT embedded in the ASM checkpoint sequencer predicate (it stays
    ``AlwaysAccept``); otherwise the derived pubkey is returned so the ASM
    checkpoint predicate requires that sequencer's signature.
    """
    sequencer_key_path = datadir / seq_fname
    ensure_priv_key(sequencer_key_path)
    sequencer_pubkey = (
        None if use_unchecked_cred_rule else generate_sequencer_pubkey(sequencer_key_path)
    )
    operator_pubkeys = get_operator_pubkeys(datadir, "bridge-operator_keys")
    return SequencerArtifacts(
        sequencer_key_path=sequencer_key_path,
        sequencer_pubkey=sequencer_pubkey,
        operator_keys=operator_pubkeys,
    )


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
        "--alpen-predicate",
        "bip340-schnorr-test",
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
    operator_pubkeys: list[str],
    ol_params_path: Path | None = None,
    sequencer_pubkey: str | None = None,
    admin_confirmation_depth: int | None = None,
) -> Path:
    params_path = datadir / "asm-params.json"
    if not operator_pubkeys:
        raise RuntimeError("gen-asm-params requires at least one operator pubkey")

    args = [
        "gen-asm-params",
        "--checkpoint-predicate",
        "bip340-schnorr-test",
        "--name",
        "ALPN",
        "--genesis-l1-height",
        str(genesis_l1_height),
        "--safe-harbour-address",
        p2tr_bosd_from_compressed_pubkey(operator_pubkeys[0]),
        "-o",
        str(params_path),
    ]
    if ol_params_path is not None:
        args.extend(["--ol-params", str(ol_params_path)])
    if sequencer_pubkey is not None:
        args.extend(["--seq-pk", sequencer_pubkey])
    if admin_confirmation_depth is not None:
        args.extend(["--confirmation-depth", str(admin_confirmation_depth)])
    for pk in operator_pubkeys:
        args.extend(["--op-pk", pk])

    run_datatool(args, bconfig)
    return params_path
