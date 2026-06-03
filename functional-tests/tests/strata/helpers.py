"""Helpers for Strata functional tests."""

import logging

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.rpc import JsonRpcClient
from envconfigs.strata import StrataEnvConfig

logger = logging.getLogger(__name__)


def assert_terminal_block_l1_update(
    rpc: JsonRpcClient,
    target_slot: int,
    min_l1_manifests_in_terminal_block: int,
) -> None:
    """Asserts a canonical terminal block exposes the expected L1 update."""

    block = rpc.strata_getBlockBySlot(target_slot)
    assert block is not None, f"terminal slot {target_slot} is missing from canonical chain"

    header = block["header"]
    assert header["slot"] == target_slot, f"unexpected terminal block header: {header}"
    assert header["is_terminal"] is True, f"slot {target_slot} is not terminal: {header}"

    manifests = block.get("manifests")
    assert manifests is not None, f"terminal slot {target_slot} has no manifests: {block}"

    manifest_count = manifests.get("manifest_count")
    assert (
        isinstance(manifest_count, int) and manifest_count >= min_l1_manifests_in_terminal_block
    ), (
        f"terminal slot {target_slot} included {manifest_count!r} L1 manifests, "
        f"expected at least {min_l1_manifests_in_terminal_block}: {block}"
    )

    status = rpc.strata_getChainStatus()
    latest = status.get("latest")
    assert isinstance(latest, dict), f"chain status missing latest epoch: {status}"
    assert latest["last_slot"] >= target_slot, (
        f"latest completed epoch did not advance past terminal slot {target_slot}: {status}"
    )

    logger.info(
        "asserted terminal slot %s with %s L1 manifests",
        target_slot,
        manifest_count,
    )


class TerminalBlockAssemblyBase(StrataNodeTest):
    """Base class for terminal block assembly checks."""

    # 64 slots at 500ms gives slower L1 mining time to include a terminal manifest.
    epoch_sealing = EpochSealingConfig.new_fixed_slot(64)

    # Pin the reorg-safe depth the manifest-burial timing relies on, so the test
    # does not silently break when the global btcio default changes.
    l1_reorg_safe_depth = 4

    l1_mining_interval_seconds: float
    terminal_blocks_to_validate: int
    terminal_slot_timeout_seconds: int
    min_l1_manifests_in_terminal_block: int

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=self.epoch_sealing,
                ol_block_time_ms=500,
                l1_reorg_safe_depth=self.l1_reorg_safe_depth,
            )
        )

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)
        rpc = strata.wait_for_rpc_ready(
            timeout=30,
            method="strata_getChainStatus",
        )

        initial_tip = rpc.strata_getChainStatus()["tip"]
        target_slot = self.epoch_sealing.next_terminal_slot_after(initial_tip["slot"])
        logger.info(
            "starting terminal block assembly check: initial_slot=%s target_slot=%s",
            initial_tip["slot"],
            target_slot,
        )

        for _ in range(self.terminal_blocks_to_validate):
            terminal_slot = target_slot
            tip = bitcoin.mine_until(
                check=lambda: rpc.strata_getChainStatus()["tip"],
                predicate=lambda value, terminal_slot=terminal_slot: (
                    value is not None and value["slot"] >= terminal_slot
                ),
                error_with=f"OL tip did not reach terminal slot {terminal_slot}",
                timeout=self.terminal_slot_timeout_seconds,
                step=self.l1_mining_interval_seconds,
                blocks_per_step=1,
            )
            logger.info(
                "tip reached slot %s while waiting for terminal slot %s",
                tip["slot"],
                terminal_slot,
            )

            assert_terminal_block_l1_update(
                rpc,
                terminal_slot,
                self.min_l1_manifests_in_terminal_block,
            )
            target_slot = self.epoch_sealing.next_terminal_slot_after(terminal_slot)

        return True
