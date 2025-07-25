import json
import os
import subprocess

from web3 import Web3


def send_tx(web3: Web3):
    """Send a simple transaction to generate activity"""
    dest = web3.to_checksum_address("deedf001900dca3ebeefdeadf001900dca3ebeef")
    txid = web3.eth.send_transaction(
        {
            "to": dest,
            "value": hex(1),
            "gas": hex(100000),
            "from": web3.address,
        }
    )
    print("txid", txid.to_0x_hex())
    web3.eth.wait_for_transaction_receipt(txid, timeout=5)


def run_dbtool_command(datadir: str, subcommand: str, *args) -> tuple[int, str, str]:
    """Run strata-dbtool command and return (return_code, stdout, stderr)"""
    cmd = ["strata-dbtool", "-d", datadir, subcommand] + list(args)
    print(f"Running command: {' '.join(cmd)}")

    result = subprocess.run(cmd, capture_output=True, text=True, cwd=os.path.dirname(datadir))

    if result.stdout:
        print(f"Stdout: {result.stdout}")
    if result.stderr:
        print(f"Stderr: {result.stderr}")

    return result.returncode, result.stdout, result.stderr


def extract_json_from_output(output: str) -> str:
    """Extract complete JSON objects from output, ignoring log lines"""
    # Find all potential JSON objects by looking for { } pairs
    start_idx = 0

    while True:
        start_idx = output.find("{", start_idx)
        if start_idx == -1:
            break

        # Count braces to find the complete JSON object
        brace_count = 0
        end_idx = -1

        for i in range(start_idx, len(output)):
            if output[i] == "{":
                brace_count += 1
            elif output[i] == "}":
                brace_count -= 1
                if brace_count == 0:
                    end_idx = i
                    break

        if end_idx != -1:
            json_str = output[start_idx : end_idx + 1]
            try:
                # Validate it's actually JSON
                json.loads(json_str)
                return json_str
            except json.JSONDecodeError:
                pass  # Not valid JSON, skip it

        start_idx = end_idx + 1 if end_idx != -1 else start_idx + 1

    return ""
