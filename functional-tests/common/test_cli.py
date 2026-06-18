"""Python wrapper for strata-test-cli commands."""

import json
import subprocess

BINARY_PATH = "strata-test-cli"


def _run_command(args: list[str]) -> str:
    """Run a CLI command and return stdout.

    Raises:
        RuntimeError: If command fails (includes stderr in message).
    """
    cmd = [BINARY_PATH] + args
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
    if result.returncode != 0:
        raise RuntimeError(
            f"strata-test-cli failed (exit {result.returncode}):\n"
            f"  cmd: {' '.join(cmd)}\n"
            f"  stderr: {result.stderr.strip()}\n"
            f"  stdout: {result.stdout.strip()}"
        )
    return result.stdout.strip()


def create_mock_deposit(
    account_serial: int,
    amount: int,
    btc_url: str,
    btc_user: str,
    btc_password: str,
) -> str:
    """Inject a deposit via the debug subprotocol.

    Returns the broadcast transaction ID (hex string).
    """
    # fmt: off
    args = [
        "create-mock-deposit",
        "--account-serial", str(account_serial),
        "--amount", str(amount),
        "--btc-url", btc_url,
        "--btc-user", btc_user,
        "--btc-password", btc_password,
    ]
    # fmt: on

    return _run_command(args)


def build_snark_withdrawal(
    target_hex: str,
    seq_no: int,
    inner_state_hex: str,
    next_inbox_idx: int,
    dest_hex: str,
    amount: int,
    fees: int = 0,
) -> dict:
    """Build a withdrawal transaction JSON.

    Returns a dict ready for strata_submitTransaction.
    """
    # fmt: off
    args = [
        "build-snark-withdrawal",
        "--target", target_hex,
        "--seq-no", str(seq_no),
        "--inner-state", inner_state_hex,
        "--next-inbox-idx", str(next_inbox_idx),
        "--dest", dest_hex,
        "--amount", str(amount),
        "--fees", str(fees),
    ]
    # fmt: on

    result = _run_command(args)
    return json.loads(result)


def _admin_signer_args(
    admin_xpriv: str | None,
    signers: list[tuple[str, int]] | None,
) -> list[str]:
    """Builds the repeatable ``--admin-xpriv`` / ``--signer-index`` CLI args.

    Two mutually exclusive shapes are supported:

    - ``admin_xpriv``: a single signer (1-of-N). ``--signer-index`` is omitted
      and defaults to member index 0 in the CLI.
    - ``signers``: a list of ``(xpriv, member_index)`` tuples for a threshold
      (M-of-N) update; one ``--admin-xpriv`` / ``--signer-index`` pair is
      emitted per signer.
    """
    if (admin_xpriv is None) == (signers is None):
        raise ValueError("provide exactly one of admin_xpriv or signers")

    if admin_xpriv is not None:
        return ["--admin-xpriv", admin_xpriv]

    args: list[str] = []
    for xpriv, index in signers:
        args += ["--admin-xpriv", xpriv, "--signer-index", str(index)]
    return args


def create_ee_predicate_update(
    seq_no: int,
    predicate: str,
    btc_url: str,
    btc_user: str,
    btc_password: str,
    admin_xpriv: str | None = None,
    signers: list[tuple[str, int]] | None = None,
    fee_rate: int = 2,
    commit_output_sats: int = 20_000,
) -> dict:
    """Broadcast an admin EE predicate update commit/reveal transaction pair.

    Pass ``admin_xpriv`` for a single-signer (1-of-N) update, or ``signers``
    (a list of ``(xpriv, member_index)`` tuples) for a threshold (M-of-N)
    update such as staging-v2's 2-of-3 admin multisig.

    Returns:
        JSON dict with commit_txid and reveal_txid.
    """
    # fmt: off
    args = [
        "create-ee-predicate-update",
        "--seq-no", str(seq_no),
        "--predicate", predicate,
        *_admin_signer_args(admin_xpriv, signers),
        "--btc-url", btc_url,
        "--btc-user", btc_user,
        "--btc-password", btc_password,
        "--fee-rate", str(fee_rate),
        "--commit-output-sats", str(commit_output_sats),
    ]
    # fmt: on

    result = _run_command(args)
    return json.loads(result)


def create_checkpoint_predicate_update(
    seq_no: int,
    predicate: str,
    btc_url: str,
    btc_user: str,
    btc_password: str,
    admin_xpriv: str | None = None,
    signers: list[tuple[str, int]] | None = None,
    fee_rate: int = 2,
    commit_output_sats: int = 20_000,
) -> dict:
    """Broadcast an admin OL checkpoint predicate update.

    Pass ``admin_xpriv`` for a single-signer (1-of-N) update, or ``signers``
    (a list of ``(xpriv, member_index)`` tuples) for a threshold (M-of-N)
    update.
    """
    # fmt: off
    args = [
        "create-checkpoint-predicate-update",
        "--seq-no", str(seq_no),
        "--predicate", predicate,
        *_admin_signer_args(admin_xpriv, signers),
        "--btc-url", btc_url,
        "--btc-user", btc_user,
        "--btc-password", btc_password,
        "--fee-rate", str(fee_rate),
        "--commit-output-sats", str(commit_output_sats),
    ]
    # fmt: on

    result = _run_command(args)
    return json.loads(result)
