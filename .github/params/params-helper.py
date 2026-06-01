#!/usr/bin/env python3
"""Merge dynamic values from datatool-generated raw params into pre-committed templates.

Templates contain static fields (operator keys, deposit_amount, etc.) with
placeholders (__PLACEHOLDER__) for dynamic fields (VKs, genesis L1 view, etc.).
Raw params from datatool have the correct dynamic values but wrong static values
(datatool defaults). This script takes the best of both.

Usage:
    params-helper.py merge --raw-dir <dir> --template-dir <dir> --output-dir <dir>
    params-helper.py extract-keys --template-dir <dir> --output-dir <dir>
"""

import argparse
import json
import sys
from pathlib import Path

ALPEN_ACCOUNT_ID = "0101010101010101010101010101010101010101010101010101010101010101"


def load_json(path: Path) -> dict:
    with open(path) as f:
        return json.load(f)


def write_json(path: Path, data: dict) -> None:
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")


def merge_rollup_params(template: dict, raw: dict) -> dict:
    template["genesis_l1_view"] = raw["genesis_l1_view"]
    template["checkpoint_predicate"] = raw["checkpoint_predicate"]
    template["evm_genesis_block_hash"] = raw["evm_genesis_block_hash"]
    template["evm_genesis_block_state_root"] = raw["evm_genesis_block_state_root"]
    return template


def merge_ol_params(template: dict, raw: dict) -> dict:
    raw_acct = raw["accounts"][ALPEN_ACCOUNT_ID]
    template["accounts"][ALPEN_ACCOUNT_ID]["predicate"] = raw_acct["predicate"]
    template["accounts"][ALPEN_ACCOUNT_ID]["inner_state"] = raw_acct["inner_state"]
    template["last_l1_block"] = raw["last_l1_block"]
    return template


def merge_asm_params(template: dict, raw: dict) -> dict:
    template["anchor"] = raw["anchor"]

    raw_checkpoint = None
    for sp in raw["subprotocols"]:
        if "Checkpoint" in sp:
            raw_checkpoint = sp["Checkpoint"]
            break

    if raw_checkpoint is None:
        print("ERROR: raw asm-params missing Checkpoint subprotocol", file=sys.stderr)
        sys.exit(1)

    for sp in template["subprotocols"]:
        if "Checkpoint" in sp:
            sp["Checkpoint"]["checkpoint_predicate"] = raw_checkpoint["checkpoint_predicate"]
            sp["Checkpoint"]["genesis_l1_height"] = raw_checkpoint["genesis_l1_height"]
            sp["Checkpoint"]["genesis_ol_blkid"] = raw_checkpoint["genesis_ol_blkid"]

    return template


def check_no_placeholders(output_dir: Path) -> bool:
    ok = True
    for name in ["rollup-params", "ol-params", "asm-params"]:
        content = (output_dir / f"{name}.json").read_text()
        if "__" in content:
            print(f"ERROR: {name}.json still has placeholders", file=sys.stderr)
            ok = False
    return ok


def cross_validate(rp: dict, ap: dict, olp: dict | None = None) -> bool:
    """Verify that shared values between rollup-params, asm-params, and ol-params are consistent."""
    ok = True

    # --- Sequencer key ---
    rp_seq_key = rp["cred_rule"]["schnorr_key"]
    ap_checkpoint = None
    for sp in ap["subprotocols"]:
        if "Checkpoint" in sp:
            ap_checkpoint = sp["Checkpoint"]
            break
    if ap_checkpoint is None:
        print("ERROR: asm-params missing Checkpoint subprotocol", file=sys.stderr)
        return False

    # sequencer_predicate is "Bip340Schnorr:<hex>"
    sp_parts = ap_checkpoint["sequencer_predicate"].split(":", 1)
    if len(sp_parts) != 2 or sp_parts[1] != rp_seq_key:
        print(
            f"ERROR: sequencer key mismatch\n"
            f"  rollup-params cred_rule.schnorr_key: {rp_seq_key}\n"
            f"  asm-params sequencer_predicate:      {ap_checkpoint['sequencer_predicate']}",
            file=sys.stderr,
        )
        ok = False

    # --- Bridge section ---
    ap_bridge = None
    for sp in ap["subprotocols"]:
        if "Bridge" in sp:
            ap_bridge = sp["Bridge"]
            break
    if ap_bridge is None:
        print("ERROR: asm-params missing Bridge subprotocol", file=sys.stderr)
        return False

    # Operator keys: rollup-params has x-only (32B), asm-params has compressed (33B with 02/03 prefix)
    rp_ops = sorted(rp.get("operators", []))
    ap_ops_xonly = sorted(op[2:] for op in ap_bridge["operators"])
    if rp_ops != ap_ops_xonly:
        print(
            f"ERROR: operator key mismatch\n"
            f"  rollup-params operators (x-only): {rp_ops}\n"
            f"  asm-params operators (stripped):   {ap_ops_xonly}",
            file=sys.stderr,
        )
        ok = False

    # Scalar fields that must match across templates
    checks = [
        ("deposit_amount / denomination", rp.get("deposit_amount"), ap_bridge.get("denomination")),
        ("recovery_delay", rp.get("recovery_delay"), ap_bridge.get("recovery_delay")),
        ("dispatch_assignment_dur / assignment_duration", rp.get("dispatch_assignment_dur"), ap_bridge.get("assignment_duration")),
    ]
    for label, rp_val, ap_val in checks:
        if rp_val != ap_val:
            print(
                f"ERROR: {label} mismatch: rollup-params={rp_val}, asm-params={ap_val}",
                file=sys.stderr,
            )
            ok = False

    # --- OL bridge_params ---
    if olp is not None:
        ol_bp = olp.get("bridge_params", {})
        ol_denom = ol_bp.get("denomination")
        if ol_denom is not None and ol_denom != rp.get("deposit_amount"):
            print(
                f"ERROR: ol-params bridge_params.denomination ({ol_denom}) "
                f"!= rollup-params deposit_amount ({rp.get('deposit_amount')})",
                file=sys.stderr,
            )
            ok = False

    return ok


def extract_seq_pk(template_dir: Path) -> str:
    rp = load_json(template_dir / "rollup-params.json")
    return rp["cred_rule"]["schnorr_key"]


def extract_operator_pks(template_dir: Path):
    ap = load_json(template_dir / "asm-params.json")
    for sp in ap["subprotocols"]:
        if "Bridge" in sp:
            return sp["Bridge"]["operators"]
    return []


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    merge_p = sub.add_parser("merge", help="Merge dynamic values from raw into templates")
    merge_p.add_argument("--raw-dir", required=True, help="Directory with datatool-generated raw params")
    merge_p.add_argument("--template-dir", required=True, help="Directory with pre-committed templates")
    merge_p.add_argument("--output-dir", required=True, help="Directory to write merged params")

    keys_p = sub.add_parser("extract-keys", help="Extract keys from templates")
    keys_p.add_argument("--template-dir", required=True, help="Directory with pre-committed templates")
    keys_p.add_argument("--output-dir", required=True, help="Directory to write key files")

    args = parser.parse_args()

    if args.command == "extract-keys":
        template_dir = Path(args.template_dir)
        output_dir = Path(args.output_dir)
        output_dir.mkdir(parents=True, exist_ok=True)

        seq_pk = extract_seq_pk(template_dir)
        print(f"  seq_pk: {seq_pk}")

        op_pks = extract_operator_pks(template_dir)
        (output_dir / "op-pks.txt").write_text("\n".join(op_pks) + "\n")
        print(f"  operators: {len(op_pks)} keys")

        (output_dir / "seq-pk.txt").write_text(seq_pk + "\n")
        return

    raw_dir = Path(args.raw_dir)
    template_dir = Path(args.template_dir)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    rp = merge_rollup_params(
        load_json(template_dir / "rollup-params.json"),
        load_json(raw_dir / "rollup-params-raw.json"),
    )
    write_json(output_dir / "rollup-params.json", rp)
    print("  rollup-params.json: merged")

    olp = merge_ol_params(
        load_json(template_dir / "ol-params.json"),
        load_json(raw_dir / "ol-params-raw.json"),
    )
    write_json(output_dir / "ol-params.json", olp)
    print("  ol-params.json: merged")

    ap = merge_asm_params(
        load_json(template_dir / "asm-params.json"),
        load_json(raw_dir / "asm-params-raw.json"),
    )
    write_json(output_dir / "asm-params.json", ap)
    print("  asm-params.json: merged")

    if not check_no_placeholders(output_dir):
        sys.exit(1)

    if not cross_validate(rp, ap, olp):
        sys.exit(1)

    print("\n  All checks passed.")


if __name__ == "__main__":
    main()
