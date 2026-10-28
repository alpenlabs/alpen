#!/usr/bin/env python3
"""Exercise admin action signatures and Bitcoin Core mempool checks on regtest.

Requires a built strata-test-cli, bitcoin-cli and bitcoind. All keys are synthetic;
all node state/artifacts live in a TemporaryDirectory removed after the node exits.
No transaction is broadcast. This does not start an ASM node or test admin activation.
"""

import argparse
import hashlib
import json
import secrets
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path


def run(command, *, input_text=None, success=True):
    result = subprocess.run(command, input=input_text, text=True, capture_output=True, timeout=45)
    if success and result.returncode:
        raise RuntimeError(f"{Path(command[0]).name} failed: {result.stderr}")
    if not success and result.returncode == 0:
        raise AssertionError("expected the command to fail")
    return result


def test_wif(secret):
    payload = b"\xef" + secret + b"\x01"
    payload += hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4]
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    value = int.from_bytes(payload, "big")
    encoded = ""
    while value:
        value, digit = divmod(value, 58)
        encoded = alphabet[digit] + encoded
    return encoded


def exercise(binary, root, rpc_base, password):
    def rpc(method, *args):
        encoded = [arg if isinstance(arg, str) else json.dumps(arg) for arg in args]
        text = run([*rpc_base, method, *encoded]).stdout.strip()
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text

    secrets_by_index = [bytes([n]) * 32 for n in (1, 2, 3)]
    addresses = []
    for secret in secrets_by_index:
        info = rpc("getdescriptorinfo", f"wpkh({test_wif(secret)})")
        assert isinstance(info, dict) and isinstance(info["descriptor"], str)
        derived = rpc("deriveaddresses", info["descriptor"])
        assert isinstance(derived, list) and len(derived) == 1
        addresses.append(derived[0])

    fixture = Path(__file__).resolve().parents[1] / "fixtures/staging_v2_asm_params.json"
    params = json.loads(fixture.read_text())
    params["anchor"]["network"] = "regtest"
    admins = [item["Admin"] for item in params["subprotocols"] if "Admin" in item]
    assert len(admins) == 1
    for role in ("strata_administrator", "strata_sequencer_manager", "alpen_administrator", "strata_security_council"):
        assert "signers" in admins[0][role] and "threshold" in admins[0][role]
        admins[0][role] = {"signers": addresses, "threshold": 2}
    params_path = root / "params.json"
    params_path.write_text(json.dumps(params))
    condition = bytes([7]) * 64
    condition_hash = hashlib.sha256(condition).hexdigest()
    action = ["--update", "ee-stf-vk", "--seq-no", "4", "--network", "regtest"]
    commitment = ["--predicate-type", "Sp1Groth16", "--predicate-hash", condition_hash]
    full = ["--predicate", "Sp1Groth16:" + condition.hex()]
    config = ["--signers-config", str(params_path)]
    text = run([binary, "print-signing-message", *action, *commitment]).stdout
    assert text == run([binary, "print-signing-message", *action, *full]).stdout
    assert not text.endswith("\n")

    # An actual Core signmessage signature, independently produced from the canonical text.
    external = rpc("signmessagewithprivkey", test_wif(secrets_by_index[0]), text)
    assert isinstance(external, str)
    local = run([binary, "sign-admin-action", *action, *commitment, *config, "--key-stdin"], input_text=secrets_by_index[2].hex()).stdout.strip()
    assert local.startswith("2:")
    for target in ("ee-stf-vk", "ol-stf-vk", "asm-stf-vk"):
        for cancel in ([], ["--cancel-target-id", "2"]):
            context = ["--update", target, "--seq-no", "4", "--network", "regtest", *cancel]
            messages = [run([binary, "print-signing-message", *context, *p]).stdout for p in (full, commitment)]
            assert messages[0] == messages[1]
            signatures = [run([binary, "sign-admin-action", *context, *p, *config, "--key-stdin"], input_text=secrets_by_index[2].hex()).stdout for p in (full, commitment)]
            assert signatures[0] == signatures[1]
    for invalid in ([], full + commitment, commitment[:2], ["--predicate", "not-a-predicate"]):
        run([binary, "print-signing-message", *action, *invalid], success=False)
        run([binary, "sign-admin-action", *action, *invalid, *config, "--key-stdin"], input_text=secrets_by_index[2].hex(), success=False)
    normalized = [f"0:{external}", local]
    run([binary, "sign-admin-action", *action, *commitment, *config, "--key-stdin"], input_text=(bytes([9]) * 32).hex(), success=False)

    rpc("createwallet", "admin-test-funding")
    wallet_rpc = [*rpc_base, "-rpcwallet=admin-test-funding"]
    mining_address = run([*wallet_rpc, "getnewaddress", "", "bech32m"]).stdout.strip()
    rpc("generatetoaddress", 101, mining_address)
    node_url = f"http://127.0.0.1:{rpc_base[2].split('=', 1)[1]}"
    funding = ["--btc-wallet", "admin-test-funding", "--btc-url", node_url, "--btc-user", "admin-smoke", "--btc-password", password, "--magic", params["magic"], "--network", "regtest", *config, "--dry-run"]
    signature_flags = [item for signature in normalized for item in ("--signature", signature)]
    created = json.loads(run([binary, "create-ee-predicate-update", "--seq-no", "4", *full, *signature_flags, *funding]).stdout)
    for signatures in ([normalized[0]], [normalized[0], normalized[0]]):
        flags = [item for signature in signatures for item in ("--signature", signature)]
        rejected = run([binary, "create-ee-predicate-update", "--seq-no", "4", *full, *flags, *funding], success=False)
        assert "admin signature check failed" in rejected.stderr
    assert created["threshold_verified"] is True
    assert created["signing_message"] == text
    accepted = rpc("testmempoolaccept", [created["commit_tx"], created["reveal_tx"]])
    assert len(accepted) == 2 and all(item.get("allowed") is True for item in accepted), accepted
    assert rpc("getrawmempool") == [], "test must not broadcast either transaction"
    bad = ["--predicate", "Sp1Groth16:" + (bytes([8]) * 64).hex()]
    rejected = run([binary, "create-ee-predicate-update", "--seq-no", "4", *bad, *signature_flags, *funding], success=False)
    assert "admin signature check failed" in rejected.stderr
    print(json.dumps({"status": "PASS", "bitcoin_core": rpc("getnetworkinfo")["version"], "valid_indices": [0, 2], "threshold_verified": True, "commit_and_reveal_mempool_allowed": True, "broadcast_transactions": 0}))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve(strict=True))
    for name in ("bitcoind", "bitcoin-cli"):
        if shutil.which(name) is None:
            raise RuntimeError(f"{name} is required")
    with tempfile.TemporaryDirectory(prefix="alpen-admin-smoke-") as directory:
        root = Path(directory)
        (root / "README.md").write_text("Transient synthetic-key regtest test artifacts, created by admin_action_smoke.py. node/: isolated Bitcoin Core state and generated cookie; params.json: synthetic role configuration; node.log: startup/exit diagnostics. Owner: invoking test. Retention: disposable after test; reproduction: rerun script. No mainnet inputs.\n")
        node = root / "node"
        node.mkdir()
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        password = secrets.token_hex(24)
        rpc_base = ["bitcoin-cli", "-regtest", f"-rpcport={port}", f"-datadir={node}", "-rpcuser=admin-smoke", f"-rpcpassword={password}"]
        with (root / "node.log").open("w") as log:
            process = subprocess.Popen(["bitcoind", "-regtest", f"-datadir={node}", "-server=1", "-listen=0", "-connect=0", "-dnsseed=0", "-discover=0", f"-rpcport={port}", "-rpcuser=admin-smoke", f"-rpcpassword={password}", "-printtoconsole=1"], stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 30
                while True:
                    result = subprocess.run([*rpc_base, "getblockchaininfo"], capture_output=True, text=True, timeout=5)
                    if result.returncode == 0:
                        assert json.loads(result.stdout)["chain"] == "regtest"
                        break
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError("isolated regtest node did not become ready: " + (root / "node.log").read_text())
                    time.sleep(0.1)
                exercise(binary, root, rpc_base, password)
            finally:
                if process.poll() is None:
                    stopped = subprocess.run([*rpc_base, "stop"], capture_output=True, text=True, timeout=5)
                    if stopped.returncode:
                        process.terminate()
                        process.wait(timeout=10)
                        raise RuntimeError("node RPC stop failed; terminated isolated test process: " + stopped.stderr)
                    process.wait(timeout=15)


if __name__ == "__main__":
    main()
