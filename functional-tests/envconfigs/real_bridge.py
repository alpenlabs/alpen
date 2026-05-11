"""EE/OL environment configured for an external real bridge."""

import json
from pathlib import Path

import flexitest

from envconfigs.el_ol import EeOLEnv


def write_no_prefund_chain_spec(ectx: flexitest.EnvContext) -> Path:
    """Write an Alpen dev chainspec with no prefunded EVM accounts."""
    repo_root = Path(__file__).resolve().parents[2]
    chain_spec_path = repo_root / "crates/reth/chainspec/src/res/alpen-dev-chain.json"
    chain_spec = json.loads(chain_spec_path.read_text())
    chain_spec["alloc"] = {}

    output_path = Path(ectx.envdd_path) / "alpen-no-prefund-chain.json"
    output_path.write_text(json.dumps(chain_spec, indent=2) + "\n")
    return output_path


class RealBridgeEeOLEnv(flexitest.EnvConfig):
    """Starts Bitcoin, Strata, and Alpen with bridge operator keys from strata-bridge."""

    def __init__(
        self,
        bridge_operator_pubkeys: list[str],
        fullnode_count: int = 1,
        pre_generate_blocks: int = 110,
        seal_epoch_slots: int | None = None,
    ):
        self.bridge_operator_pubkeys = bridge_operator_pubkeys
        self.fullnode_count = fullnode_count
        self.pre_generate_blocks = pre_generate_blocks
        self.seal_epoch_slots = seal_epoch_slots

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        chain_spec = write_no_prefund_chain_spec(ectx)
        return EeOLEnv(
            fullnode_count=self.fullnode_count,
            pre_generate_blocks=self.pre_generate_blocks,
            seal_epoch_slots=self.seal_epoch_slots,
            bridge_operator_pubkeys=self.bridge_operator_pubkeys,
            custom_chain=str(chain_spec),
            alpen_chain_config=chain_spec,
        ).init(ectx)
