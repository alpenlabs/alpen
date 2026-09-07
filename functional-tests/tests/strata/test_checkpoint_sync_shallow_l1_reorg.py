"""Checkpoint observations follow the canonical L1 branch across shallow reorgs."""

from dataclasses import dataclass

import flexitest

from common.base_test import BaseTest
from common.config.constants import ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.wait import wait_until, wait_until_with_value
from envconfigs.checkpoint_sync import CheckpointSyncEnv
from tests.checkpoint.helpers import (
    CHECKPOINT_SUBPROTOCOL_ID,
    OL_STF_CHECKPOINT_TX_TYPE,
    extract_posted_checkpoint_payload,
    parse_checkpoint_payload,
)

REORG_SAFE_DEPTH = 4
TARGET_EPOCH = 1


@dataclass(frozen=True)
class _CheckpointInclusion:
    """Identifies a checkpoint transaction and its first L1 inclusion."""

    txid: str
    block_hash: str
    height: int
    manifest: object


@flexitest.register
class TestCheckpointSyncShallowL1Reorg(BaseTest):
    """Verifies an orphaned checkpoint becomes pending and can recover."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            CheckpointSyncEnv(
                pre_generate_blocks=110,
                seal_epoch_slots=4,
                ol_block_time_ms=750,
                l1_reorg_safe_depth=REORG_SAFE_DEPTH,
            )
        )

    def main(self, ctx):
        """Runs the checkpoint through inclusion, reorg, and recovery."""

        sequencer: StrataService = self.get_service(ServiceType.Strata)
        css: StrataService = self.get_service(ServiceType.StrataCheckpointNode)
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        btc_rpc = bitcoin.create_rpc()

        seq_rpc = sequencer.wait_for_rpc_ready(timeout=20)
        css.wait_for_rpc_ready(timeout=20)

        scenario = _ShallowReorgScenario(sequencer, css, btc_rpc, seq_rpc)
        checkpoint = scenario.include_checkpoint()
        scenario.orphan_checkpoint(checkpoint)
        scenario.verify_orphan_is_ignored(checkpoint)
        scenario.reinclude_checkpoint(checkpoint)
        scenario.advance_to_finality()
        scenario.verify_checkpoint_is_finalized()


class _ShallowReorgScenario:
    """Drives a checkpoint through a controlled shallow L1 reorg."""

    def __init__(self, sequencer, css, btc_rpc, seq_rpc):
        """Initializes the scenario with its node and Bitcoin adapters."""

        self.sequencer = sequencer
        self.css = css
        self.btc_rpc = btc_rpc
        self.seq_rpc = seq_rpc
        self.mining_address = btc_rpc.proxy.getnewaddress()

    def include_checkpoint(self) -> _CheckpointInclusion:
        """Includes the target checkpoint with exactly one L1 confirmation."""

        bootstrap_hash = self.btc_rpc.proxy.generateblock(self.mining_address, [])["hash"]
        bootstrap_height = self.btc_rpc.proxy.getblock(bootstrap_hash)["height"]
        self._wait_for_sequencer_manifest(bootstrap_height)

        checkpoint_txid = wait_until_with_value(
            self._find_checkpoint_txid_in_mempool,
            lambda txid: txid is not None,
            error_with="target checkpoint transaction did not reach the L1 mempool",
            timeout=120,
        )
        assert checkpoint_txid is not None

        checkpoint_block_hash = self.btc_rpc.proxy.generatetoaddress(1, self.mining_address)[0]
        checkpoint_info = self._wait_for_checkpoint_status(
            "confirmed", "checkpoint was not observed after its L1 inclusion"
        )
        l1_reference = checkpoint_info["confirmation_status"]["l1_reference"]
        assert l1_reference["txid"] == checkpoint_txid
        checkpoint_height = l1_reference["l1_block"]["height"]
        manifest = self.sequencer.wait_for_asm_manifest_commitment_at(
            checkpoint_height, rpc=self.seq_rpc, timeout=120
        )

        return _CheckpointInclusion(
            txid=checkpoint_txid,
            block_hash=checkpoint_block_hash,
            height=checkpoint_height,
            manifest=manifest,
        )

    def orphan_checkpoint(self, checkpoint: _CheckpointInclusion) -> None:
        """Orphans the checkpoint by replacing its block with an empty branch."""

        self.btc_rpc.proxy.invalidateblock(checkpoint.block_hash)
        wait_until(
            lambda: checkpoint.txid in self.btc_rpc.proxy.getrawmempool(),
            error_with="orphaned checkpoint transaction did not return to the mempool",
            timeout=30,
        )
        assert self.btc_rpc.proxy.getblockcount() == checkpoint.height - 1

        for _ in range(REORG_SAFE_DEPTH):
            mined_hash = self.btc_rpc.proxy.generateblock(self.mining_address, [])["hash"]
            mined_block = self.btc_rpc.proxy.getblock(mined_hash)
            assert len(mined_block["tx"]) == 1, "replacement block was not coinbase-only"
            self._wait_for_sequencer_manifest(mined_block["height"])

    def verify_orphan_is_ignored(self, checkpoint: _CheckpointInclusion) -> None:
        """Verifies RPC and checkpoint sync reject the orphaned observation."""

        self.sequencer.wait_for_asm_manifest_commitment_at(
            checkpoint.height,
            rpc=self.seq_rpc,
            timeout=120,
            differs_from=checkpoint.manifest,
        )
        self._wait_for_checkpoint_status(
            "pending", "orphaned checkpoint observation remained confirmed"
        )
        wait_until(
            lambda: self.seq_rpc.strata_getChainStatus()["confirmed"]["epoch"] < TARGET_EPOCH,
            error_with="CSM retained the orphaned checkpoint",
            timeout=60,
        )
        replacement_tip_height = self.btc_rpc.proxy.getblockcount()
        self.css.wait_for_asm_manifest_commitment_at(replacement_tip_height, timeout=60)
        assert self.css.get_sync_status()["finalized"]["epoch"] < TARGET_EPOCH, (
            "checkpoint-sync node applied the orphaned checkpoint"
        )

    def reinclude_checkpoint(self, checkpoint: _CheckpointInclusion) -> None:
        """Includes the orphaned checkpoint again and waits for confirmation."""

        recovery_hash = self.btc_rpc.proxy.generatetoaddress(1, self.mining_address)[0]
        assert checkpoint.txid in self.btc_rpc.proxy.getblock(recovery_hash)["tx"]
        self._wait_for_sequencer_manifest(self.btc_rpc.proxy.getblockcount())
        self._wait_for_checkpoint_status(
            "confirmed", "canonical checkpoint was not observed after the reorg"
        )

    def advance_to_finality(self) -> None:
        """Advances L1 until the recovered checkpoint reaches safe depth."""

        for _ in range(REORG_SAFE_DEPTH - 1):
            self.btc_rpc.proxy.generatetoaddress(1, self.mining_address)
        final_height = self.btc_rpc.proxy.getblockcount()
        self.css.wait_for_asm_manifest_commitment_at(final_height, timeout=60)

    def verify_checkpoint_is_finalized(self) -> None:
        """Verifies CSS application and finalized RPC status after recovery."""

        wait_until_with_value(
            self.css.get_sync_status,
            lambda status: status["finalized"]["epoch"] >= TARGET_EPOCH,
            error_with="checkpoint-sync node did not apply the canonical checkpoint",
            timeout=120,
        )
        self._wait_for_checkpoint_status(
            "finalized", "canonical checkpoint did not reach finalized RPC status"
        )

    def _wait_for_sequencer_manifest(self, height: int) -> None:
        """Waits until the sequencer exposes an ASM-manifest commitment at an L1 height."""

        self.sequencer.wait_for_asm_manifest_commitment_at(height, rpc=self.seq_rpc, timeout=60)

    def _wait_for_checkpoint_status(self, expected_status: str, error_with: str) -> dict:
        """Waits until the target checkpoint reaches the expected RPC status."""

        checkpoint_info = wait_until_with_value(
            lambda: self.seq_rpc.strata_getCheckpointInfo(TARGET_EPOCH),
            lambda info: (
                info is not None and info["confirmation_status"]["status"] == expected_status
            ),
            error_with=error_with,
            timeout=60,
        )
        assert checkpoint_info is not None
        return checkpoint_info

    def _find_checkpoint_txid_in_mempool(self) -> str | None:
        """Finds the target checkpoint transaction in the L1 mempool."""

        for txid in self.btc_rpc.proxy.getrawmempool():
            tx = self.btc_rpc.proxy.getrawtransaction(txid, 1)
            script = bytes.fromhex(tx["vout"][0]["scriptPubKey"]["hex"])
            if len(script) < 8 or script[0] != 0x6A:
                continue

            tag = script[2:]
            if tag[4:6] != bytes([CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE]):
                continue

            payload = extract_posted_checkpoint_payload(self.btc_rpc, txid)
            if parse_checkpoint_payload(payload).epoch == TARGET_EPOCH:
                return txid

        return None
