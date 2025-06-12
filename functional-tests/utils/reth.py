import json
import os

current_dir = os.path.dirname(os.path.abspath(__file__))

CHAIN_CONFIGS = {
    "dev": os.path.join(
        current_dir, "..", "..", "crates", "reth", "chainspec", "src", "res", "alpen-dev-chain.json"
    ),
    "devnet": os.path.join(
        current_dir, "..", "..", "crates", "reth", "chainspec", "src", "res", "devnet-chain.json"
    ),
}


def get_chainconfig(config_or_path: str = "dev"):
    json_path = CHAIN_CONFIGS.get(config_or_path, config_or_path)

    # Open and load the JSON data
    with open(json_path) as file:
        data = json.load(file)

    return data
