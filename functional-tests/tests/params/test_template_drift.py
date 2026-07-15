"""Verify CI params templates stay in sync with datatool output schema.

Generates params locally using strata-datatool and compares the field
structure against the pre-committed templates in .github/params/templates/.
If datatool adds or removes fields, this test fails and tells you which
templates need updating.
"""

import json
import tempfile
from pathlib import Path

import flexitest

from common.base_test import BaseTest
from common.datatool import run_datatool

REPO_ROOT = Path(__file__).resolve().parents[3]
TEMPLATES_DIR = REPO_ROOT / ".github" / "params" / "templates"
L1_ANCHOR = REPO_ROOT / ".github" / "fixtures" / "l1-anchor.json"
CHAIN_CONFIG = REPO_ROOT / "crates" / "reth" / "chainspec" / "src" / "res" / "alpen-dev-chain.json"


PLACEHOLDER_PREFIX = "__"


class NoServicesEnv(flexitest.EnvConfig):
    """Environment for tests that only need local binaries."""

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        return flexitest.LiveEnv({})


def collect_keys(obj, prefix=""):
    """Recursively collect all key paths from a JSON object."""
    keys = set()
    if isinstance(obj, dict):
        for k, v in obj.items():
            path = f"{prefix}.{k}" if prefix else k
            keys.add(path)
            keys |= collect_keys(v, path)
    elif isinstance(obj, list) and obj and isinstance(obj[0], dict):
        for item in obj:
            keys |= collect_keys(item, prefix + "[]")
    return keys


def collect_keys_skipping_placeholders(obj, prefix=""):
    """Collect key paths, stopping at placeholder values.

    Placeholder fields (string values starting with __) contribute their
    own key but not any nested keys — the placeholder replaces a subtree
    that will be filled by datatool at generation time.
    """
    keys = set()
    if isinstance(obj, dict):
        for k, v in obj.items():
            path = f"{prefix}.{k}" if prefix else k
            keys.add(path)
            if isinstance(v, str) and v.startswith(PLACEHOLDER_PREFIX):
                continue  # stop recursion — subtree is a placeholder
            keys |= collect_keys_skipping_placeholders(v, path)
    elif isinstance(obj, list) and obj and isinstance(obj[0], dict):
        for item in obj:
            keys |= collect_keys_skipping_placeholders(item, prefix + "[]")
    return keys


def collect_placeholder_keys(obj, prefix=""):
    """Return top-level keys whose value is a placeholder string."""
    keys = set()
    if isinstance(obj, dict):
        for k, v in obj.items():
            path = f"{prefix}.{k}" if prefix else k
            if isinstance(v, str) and v.startswith(PLACEHOLDER_PREFIX):
                keys.add(path)
            else:
                keys |= collect_placeholder_keys(v, path)
    elif isinstance(obj, list) and obj and isinstance(obj[0], dict):
        for item in obj:
            keys |= collect_placeholder_keys(item, prefix + "[]")
    return keys


def generate_raw_params(tmpdir):
    """Generate ee-params, ol-params, and asm-params using local datatool with fixture L1 anchor."""
    ee_path = Path(tmpdir) / "ee-params.json"
    ol_path = Path(tmpdir) / "ol-params.json"
    asm_path = Path(tmpdir) / "asm-params.json"

    run_datatool(
        [
            "gen-ee-params",
            "-o",
            str(ee_path),
            "--alpen-chain-config",
            str(CHAIN_CONFIG),
            "--bridge-denomination-sats",
            "200000000",
            "--max-withdrawal-amount-sats",
            "1000000000",
            "--max-withdrawal-descriptor-len",
            "81",
        ]
    )

    assert ee_path.exists(), f"ee-params not generated at {ee_path}"

    run_datatool(
        [
            "gen-ol-params",
            "--l1-anchor-file",
            str(L1_ANCHOR),
            "--alpen-predicate",
            "bip340-schnorr-test",
            "--ee-params",
            str(ee_path),
            "-o",
            str(ol_path),
        ]
    )

    assert ol_path.exists(), f"ol-params not generated at {ol_path}"

    # gen-asm-params requires at least one operator key
    dummy_op_pk = "02" + "ab" * 32
    dummy_safe_harbour = "04" + "ab" * 32
    run_datatool(
        [
            "gen-asm-params",
            "--l1-anchor-file",
            str(L1_ANCHOR),
            "--ol-params",
            str(ol_path),
            "--checkpoint-predicate",
            "bip340-schnorr-test",
            "--name",
            "ALPN",
            "--op-pk",
            dummy_op_pk,
            "--safe-harbour-address",
            dummy_safe_harbour,
            "-o",
            str(asm_path),
        ]
    )

    assert asm_path.exists(), f"asm-params not generated at {asm_path}"

    return {
        "ee-params": ee_path,
        "ol-params": ol_path,
        "asm-params": asm_path,
    }


@flexitest.register
class TestParamsTemplateDrift(BaseTest):
    """Detects field drift between datatool output and CI params templates."""

    def __init__(self, ctx: flexitest.InitContext):
        # No services needed — just datatool on PATH
        ctx.set_env(NoServicesEnv())

    def main(self, ctx):
        with tempfile.TemporaryDirectory() as tmpdir:
            raw_params = generate_raw_params(tmpdir)
            errors = []

            for env_dir in TEMPLATES_DIR.iterdir():
                if not env_dir.is_dir():
                    continue
                env_name = env_dir.name

                for param_name, raw_path in raw_params.items():
                    template_path = env_dir / f"{param_name}.json"
                    if not template_path.exists():
                        errors.append(f"{env_name}/{param_name}.json: template missing")
                        continue

                    with open(raw_path) as f:
                        raw_data = json.load(f)
                    with open(template_path) as f:
                        tpl_data = json.load(f)

                    raw_keys = collect_keys(raw_data)
                    tpl_keys = collect_keys_skipping_placeholders(tpl_data)
                    placeholder_keys = collect_placeholder_keys(tpl_data)

                    # For non-placeholder fields, compare exact key structure
                    comparable_raw = {
                        k
                        for k in raw_keys
                        if not any(k.startswith(p + ".") or k == p for p in placeholder_keys)
                    }
                    comparable_tpl = tpl_keys - placeholder_keys

                    missing_in_template = comparable_raw - comparable_tpl
                    extra_in_template = comparable_tpl - comparable_raw

                    # For placeholder fields, just verify top-level key exists in raw
                    missing_placeholders = placeholder_keys - raw_keys

                    tag = f"{env_name}/{param_name}.json"
                    if missing_in_template:
                        errors.append(
                            f"{tag}: new fields in datatool:\n  {sorted(missing_in_template)}"
                        )
                    if extra_in_template:
                        errors.append(
                            f"{tag}: removed from datatool:\n  {sorted(extra_in_template)}"
                        )
                    if missing_placeholders:
                        errors.append(
                            f"{tag}: placeholder key gone:\n  {sorted(missing_placeholders)}"
                        )

            if errors:
                msg = "Template drift detected. Update templates to match datatool output:\n\n"
                msg += "\n\n".join(errors)
                raise AssertionError(msg)

            return True
