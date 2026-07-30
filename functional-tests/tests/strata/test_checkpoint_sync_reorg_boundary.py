"""Checkpoint sync applies an L1 checkpoint only at the reorg-safe boundary."""

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until_with_value
from envconfigs.el_ol_checkpoint_sync import EeOLCheckpointSyncEnv

REORG_SAFE_DEPTH = 4
TARGET_EPOCH = 1


@flexitest.register
class TestCheckpointSyncReorgBoundary(BaseTest):
    """CSS defers a confirmed checkpoint until it is buried by exactly four blocks."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            EeOLCheckpointSyncEnv(
                pre_generate_blocks=110,
                seal_epoch_slots=4,
                ol_block_time_ms=750,
                l1_reorg_safe_depth=REORG_SAFE_DEPTH,
            )
        )

    def main(self, ctx):
        sequencer: StrataService = self.get_service(ServiceType.Strata)
        checkpoint_node: StrataService = self.get_service(ServiceType.StrataCheckpointNode)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        seq_rpc = sequencer.wait_for_rpc_ready(timeout=20)
        checkpoint_node.wait_for_rpc_ready(timeout=20)

        checkpoint_info = wait_until_with_value(
            lambda: _mine_and_get_checkpoint_info(seq_rpc, btc_rpc),
            lambda info: info is not None and info["confirmation_status"]["status"] == "confirmed",
            error_with="checkpoint was not observed on L1 before reaching finality",
            timeout=120,
        )
        l1_height = checkpoint_info["confirmation_status"]["l1_reference"]["l1_block"]["height"]
        tip_height = btc_rpc.proxy.getblockcount()
        observed_depth = tip_height - l1_height + 1
        assert observed_depth < REORG_SAFE_DEPTH, (
            f"checkpoint unexpectedly reorg-safe at depth {observed_depth}"
        )

        # Check CSS at every sub-safe confirmation depth. Wait until CSS has
        # processed the just-mined L1 height first; otherwise this could read a
        # stale status and hide an early, incorrect application.
        checkpoint_node.wait_for_asm_manifest_commitment_at(tip_height, timeout=60)
        css_status = checkpoint_node.get_sync_status()
        assert css_status["finalized"]["epoch"] < TARGET_EPOCH, (
            "CSS applied a checkpoint before the configured reorg-safe depth"
        )
        while observed_depth < REORG_SAFE_DEPTH:
            btc_rpc.proxy.generatetoaddress(1, btc_rpc.proxy.getnewaddress())
            observed_depth += 1
            current_height = btc_rpc.proxy.getblockcount()
            checkpoint_node.wait_for_asm_manifest_commitment_at(current_height, timeout=60)
            if observed_depth < REORG_SAFE_DEPTH:
                css_status = checkpoint_node.get_sync_status()
                assert css_status["finalized"]["epoch"] < TARGET_EPOCH, (
                    "CSS applied a checkpoint before the configured reorg-safe depth"
                )

        boundary_status = wait_until_with_value(
            checkpoint_node.get_sync_status,
            lambda status: status["finalized"]["epoch"] >= TARGET_EPOCH,
            error_with="CSS did not apply the checkpoint at the reorg-safe boundary",
            timeout=120,
        )
        assert boundary_status["finalized"]["epoch"] == TARGET_EPOCH


def _mine_and_get_checkpoint_info(seq_rpc, btc_rpc) -> dict | None:
    btc_rpc.proxy.generatetoaddress(1, btc_rpc.proxy.getnewaddress())
    return seq_rpc.strata_getCheckpointInfo(TARGET_EPOCH)
