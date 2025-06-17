import requests
import logging
import argparse
import time
import pprint

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(levelname)s - %(name)s - %(message)s',
    datefmt='%Y-%m-%d %H:%M:%S'
)

# REMOTE_URL = "https://rpc.testnet.alpenlabs.io/"

def parse_args():
    parser = argparse.ArgumentParser(description="Fetch sync status from strata client.")
    parser.add_argument(
        '--url',
        type=str,
        default="http://localhost:8432",
        help='The JSON-RPC endpoint URL to connect to.'
    )
    parser.add_argument(
        '--compare-url',
        type=str,
        help='Optional second JSON-RPC endpoint to compare finalized_epoch against.'
    )
    return parser.parse_args()

def make_jsonrpc_request(url: str, method: str, params=None, req_id: int = 0) -> dict | None:
    """Send a JSON-RPC 2.0 request and return only the result."""
    if params is None:
        params = []

    payload = {
        "jsonrpc": "2.0",
        "method": method,
        "id": req_id,
        "params": params,
    }

    headers = {"Content-Type": "application/json"}

    try:
        logging.info(f"{method=} {url=}")
        response = requests.post(url, json=payload, headers=headers, timeout=10)
        response.raise_for_status()
        data = response.json()
        return data.get("result")  # Extract and return only the 'result' field
    except requests.exceptions.RequestException as e:
        print(f"[!] Request failed: {e}")
    except (ValueError, KeyError) as e:
        print(f"[!] Failed to parse response or missing 'result': {e}")


def get_sync_status(url: str) -> dict | None:
    return make_jsonrpc_request(url, "strata_syncStatus")


def get_raw_chainstate_raw(url: str, slot: int):
    return make_jsonrpc_request(url, "strata_getChainstateRaw",[slot])


if __name__ == "__main__":
    args = parse_args()
    
    current_epoch = None
    poll_delay = 1

    while True:
        sync_status = get_sync_status(args.url)
        compare_status = get_sync_status(args.compare_url) if args.compare_url else None

        if sync_status:
            finalized_epoch = sync_status["observed_finalized_epoch"]["epoch"]
            if finalized_epoch != current_epoch:
                finalized_block_id = sync_status["finalized_block_id"]
                final_slot = sync_status["observed_finalized_epoch"]["last_slot"]
                logging.info(f"epoch finalized: {finalized_epoch=} {finalized_block_id=} {final_slot=}")
                pprint.pprint(sync_status)

                if compare_status:
                    chainstate = get_raw_chainstate_raw(args.url, final_slot)
                    chainstate_to_compare = get_raw_chainstate_raw(args.compare_url, final_slot)
                    assert chainstate == chainstate_to_compare, f"chainstate mismatch slot={final_slot}"
                    logging.info(f"chainstate match with {args.compare_url} for slot={final_slot}")

                current_epoch = finalized_epoch
                poll_delay = 1  # Reset delay
            else:
                logging.info(f"no new epoch finalized: {current_epoch=}, waiting {poll_delay}s before polling again...")
                poll_delay = min(poll_delay * 2, 60)  # Cap at 60 seconds
        
        time.sleep(poll_delay)