#!/usr/bin/env python3
"""Merge dynamic values from datatool-generated raw params into pre-committed templates.

Templates contain static fields (operator keys, deposit_amount, etc.) with
placeholders (__PLACEHOLDER__) for dynamic fields (VKs, genesis L1 anchor, etc.).
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
    for name in ["ol-params", "asm-params"]:
        content = (output_dir / f"{name}.json").read_text()
        if "__" in content:
            print(f"ERROR: {name}.json still has placeholders", file=sys.stderr)
            ok = False
    return ok


def cross_validate(ap: dict, olp: dict) -> bool:
    """Verify that shared values between asm-params and ol-params are consistent."""
    ok = True

    ap_bridge = None
    for sp in ap["subprotocols"]:
        if "Bridge" in sp:
            ap_bridge = sp["Bridge"]
            break
    if ap_bridge is None:
        print("ERROR: asm-params missing Bridge subprotocol", file=sys.stderr)
        return False

    # The OL withdrawal denomination must match the ASM bridge denomination.
    ol_denom = olp.get("bridge_params", {}).get("denomination")
    ap_denom = ap_bridge.get("denomination")
    if ol_denom is not None and ap_denom is not None and ol_denom != ap_denom:
        print(
            f"ERROR: denomination mismatch: "
            f"ol-params bridge_params.denomination={ol_denom}, "
            f"asm-params Bridge.denomination={ap_denom}",
            file=sys.stderr,
        )
        ok = False

    return ok


def extract_operator_pks(template_dir: Path):
    ap = load_json(template_dir / "asm-params.json")
    for sp in ap["subprotocols"]:
        if "Bridge" in sp:
            return sp["Bridge"]["operators"]
    return []


def extract_safe_harbour(template_dir: Path) -> str:
    ap = load_json(template_dir / "asm-params.json")
    for sp in ap["subprotocols"]:
        if "Bridge" in sp:
            return sp["Bridge"]["safe_harbour_address"]
    raise ValueError("safe_harbour_address not found in asm-params template")


def align_denomination(raw_path: Path, template_path: Path, out_path: Path) -> int:
    """Write a copy of the datatool raw ol-params with bridge_params (denomination
    + cap) taken from the template, to out_path.

    The OL genesis STF hashes bridge_params into the genesis block, so the
    genesis_ol_blkid that gen-asm-params derives from the raw is
    denomination-dependent. gen-ol-params has no denomination flag (it emits the
    datatool default), so without this the deployed ol-params denomination (from
    the template) could differ from the value baked into genesis_ol_blkid and the
    node would fail to bootstrap. Feeding this aligned copy to
    gen-asm-params --ol-params keeps genesis_ol_blkid, the deployed ol-params, and
    asm Bridge.denomination (via --deposit-sats) in agreement.

    Writes to a new out_path rather than patching raw_path in place: the raw is
    produced by datatool running as root in docker, so it is not writable by the
    host runner user. Prints the denomination (sats) on stdout for the caller; all
    diagnostics go to stderr so stdout stays a single clean integer.
    """
    tpl_bridge = load_json(template_path)["bridge_params"]
    denom = int(tpl_bridge["denomination"])
    maxw = tpl_bridge.get("max_withdrawal_amount")

    # Mirror BridgeParams::new invariants (crates/bridge-params) to fail early
    # instead of letting datatool reject the aligned ol-params mid-run.
    if denom <= 0:
        print("ERROR: template bridge denomination must be non-zero", file=sys.stderr)
        sys.exit(1)
    if maxw is not None and (maxw < denom or maxw % denom != 0):
        print(
            f"ERROR: max_withdrawal_amount {maxw} must be >= and a multiple of "
            f"denomination {denom}",
            file=sys.stderr,
        )
        sys.exit(1)

    raw = load_json(raw_path)
    raw["bridge_params"] = {"denomination": denom, "max_withdrawal_amount": maxw}
    write_json(out_path, raw)
    print(f"  aligned ol-params bridge_params <- denomination={denom}, max_withdrawal_amount={maxw}", file=sys.stderr)
    print(denom)
    return denom


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

    align_p = sub.add_parser(
        "align-denomination",
        help="Patch raw ol-params bridge denomination from template; print denomination (sats)",
    )
    align_p.add_argument("--raw", required=True, help="Path to datatool raw ol-params.json (read-only)")
    align_p.add_argument("--template", required=True, help="Path to template ol-params.json (denomination source)")
    align_p.add_argument("--out", required=True, help="Path to write the aligned ol-params.json")

    args = parser.parse_args()

    if args.command == "align-denomination":
        align_denomination(Path(args.raw), Path(args.template), Path(args.out))
        return

    if args.command == "extract-keys":
        template_dir = Path(args.template_dir)
        output_dir = Path(args.output_dir)
        output_dir.mkdir(parents=True, exist_ok=True)

        op_pks = extract_operator_pks(template_dir)
        (output_dir / "op-pks.txt").write_text("\n".join(op_pks) + "\n")
        print(f"  operators: {len(op_pks)} keys")

        safe_harbour = extract_safe_harbour(template_dir)
        (output_dir / "safe-harbour.txt").write_text(safe_harbour + "\n")
        print(f"  safe_harbour_address: {safe_harbour}")
        return

    raw_dir = Path(args.raw_dir)
    template_dir = Path(args.template_dir)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

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

    if not cross_validate(ap, olp):
        sys.exit(1)

    print("\n  All checks passed.")


if __name__ == "__main__":
    main()
