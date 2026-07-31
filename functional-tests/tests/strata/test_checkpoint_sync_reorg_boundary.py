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
    """CSS applies a real L1 checkpoint after it reaches the reorg-safe boundary."""

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
            lambda: _mine_and_get_checkpoint_info(sequencer, seq_rpc, btc_rpc),
            lambda info: info is not None and info["confirmation_status"]["status"] == "confirmed",
            error_with="checkpoint was not observed on L1 before reaching finality",
            timeout=120,
            step=0,
        )
        l1_height = checkpoint_info["confirmation_status"]["l1_reference"]["l1_block"]["height"]
        tip_height = btc_rpc.proxy.getblockcount()
        observed_depth = tip_height - l1_height + 1
        assert observed_depth < REORG_SAFE_DEPTH, (
            f"checkpoint unexpectedly reorg-safe at depth {observed_depth}"
        )

        # The service-level test covers rejection at every unsafe CSM update.
        # This full-stack test instead drives the actual L1/ASM/CSM pipeline to
        # the boundary and verifies that CSS converges there. The public RPC
        # exposes OL status, not CSM-consumer progress, so it cannot make a
        # reliable per-update negative assertion while this pipeline is async.
        while observed_depth < REORG_SAFE_DEPTH:
            btc_rpc.proxy.generatetoaddress(1, btc_rpc.proxy.getnewaddress())
            observed_depth += 1
            current_height = btc_rpc.proxy.getblockcount()
            checkpoint_node.wait_for_asm_manifest_commitment_at(current_height, timeout=60)

        boundary_status = wait_until_with_value(
            checkpoint_node.get_sync_status,
            lambda status: status["finalized"]["epoch"] >= TARGET_EPOCH,
            error_with="CSS did not apply the checkpoint at the reorg-safe boundary",
            timeout=120,
        )
        assert boundary_status["finalized"]["epoch"] == TARGET_EPOCH


def _mine_and_get_checkpoint_info(sequencer, seq_rpc, btc_rpc) -> dict | None:
    checkpoint_info = seq_rpc.strata_getCheckpointInfo(TARGET_EPOCH)
    status = None if checkpoint_info is None else checkpoint_info["confirmation_status"]["status"]
    assert status != "finalized", "checkpoint reached finality before the test observed it"
    if status == "confirmed":
        return checkpoint_info

    btc_rpc.proxy.generatetoaddress(1, btc_rpc.proxy.getnewaddress())
    sequencer.wait_for_asm_manifest_commitment_at(btc_rpc.proxy.getblockcount(), timeout=30)
    return checkpoint_info
