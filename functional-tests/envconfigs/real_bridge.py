"""EE/OL environment configured for an external real bridge."""

import json
import os
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
        bridge_operator_xprivs: list[str],
        fullnode_count: int = 1,
        pre_generate_blocks: int = 110,
        seal_epoch_slots: int | None = None,
    ):
        self.bridge_operator_xprivs = bridge_operator_xprivs
        self.fullnode_count = fullnode_count
        self.pre_generate_blocks = pre_generate_blocks
        self.seal_epoch_slots = seal_epoch_slots

    def init(self, ectx: flexitest.EnvContext) -> flexitest.LiveEnv:
        chain_spec = write_no_prefund_chain_spec(ectx)
        old_operator_keys = os.environ.get("ALPEN_BRIDGE_OPERATOR_XPRIVS_JSON")
        os.environ["ALPEN_BRIDGE_OPERATOR_XPRIVS_JSON"] = json.dumps(self.bridge_operator_xprivs)
        try:
            return EeOLEnv(
                fullnode_count=self.fullnode_count,
                pre_generate_blocks=self.pre_generate_blocks,
                seal_epoch_slots=self.seal_epoch_slots,
                ol_block_time_ms=_env_int("ALPEN_REAL_BRIDGE_OL_BLOCK_TIME_MS"),
                dev_track_latest_epoch=_env_flag("ALPEN_REAL_BRIDGE_DEV_TRACK_FINALIZED_EPOCH"),
                batch_sealing_block_count=_env_int(
                    "ALPEN_REAL_BRIDGE_BATCH_SEALING_BLOCK_COUNT", 10
                ),
                custom_chain=str(chain_spec),
            ).init(ectx)
        finally:
            if old_operator_keys is None:
                os.environ.pop("ALPEN_BRIDGE_OPERATOR_XPRIVS_JSON", None)
            else:
                os.environ["ALPEN_BRIDGE_OPERATOR_XPRIVS_JSON"] = old_operator_keys


def _env_int(name: str, default: int | None = None) -> int | None:
    value = os.environ.get(name)
    if value is None:
        return default
    return int(value)


def _env_flag(name: str) -> bool:
    return os.environ.get(name, "").lower() in ("1", "true", "yes", "on")
