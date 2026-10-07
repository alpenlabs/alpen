"""STR-4488: the V0 sequencer seals and halts at an OL predicate enactment."""

import logging
import time
from pathlib import Path

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.test_cli import create_checkpoint_predicate_update
from common.wait import wait_until_with_value
from envconfigs.strata import StrataEnvConfig
from tests.checkpoint.helpers import (
    mine_until_finalized_epoch,
)

logger = logging.getLogger(__name__)

ADMIN_CONFIRMATION_DEPTH = 2
HALT_OBSERVATION_L1_BLOCKS = 3
PREDICATE_SETTLE_TIMEOUT_SECONDS = 120
RESTART_PAUSE_SECONDS = 2
RESTART_TIMEOUT_SECONDS = 30


@flexitest.register
class TestCheckpointPredicateRotation(StrataNodeTest):
    """An OL predicate enactment seals its block and halts V0 sequencing."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=EpochSealingConfig(slots_per_epoch=4),
                fund_test_cli_wallet=True,
                admin_confirmation_depth=ADMIN_CONFIRMATION_DEPTH,
                l1_reorg_safe_depth=1,
            )
        )

    def main(self, ctx):
        bitcoin: BitcoinService = self.get_service(ServiceType.Bitcoin)
        strata: StrataService = self.get_service(ServiceType.Strata)

        btc_rpc = bitcoin.create_rpc()
        strata_rpc = strata.wait_for_rpc_ready(timeout=20)
        mine_addr = btc_rpc.proxy.getnewaddress()

        baseline = mine_until_finalized_epoch(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            target_epoch=1,
            timeout=120,
            step=1.0,
        )
        logger.info("baseline finalized epoch under the initial predicate: %s", baseline["epoch"])

        admin_xpriv = self._read_admin_xpriv(strata)
        result = create_checkpoint_predicate_update(
            seq_no=1,
            predicate="NeverAccept",
            admin_xpriv=admin_xpriv,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        logger.info("submitted NeverAccept checkpoint predicate update: %s", result)

        self._mine_l1_and_wait_for_asm(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            btc_rpc=btc_rpc,
            mine_addr=mine_addr,
            blocks=1,
            timeout=PREDICATE_SETTLE_TIMEOUT_SECONDS,
        )

        reveal = btc_rpc.proxy.getrawtransaction(result["reveal_txid"], True)
        inclusion_height = btc_rpc.proxy.getblockheader(reveal["blockhash"])["height"]
        boundary = inclusion_height + ADMIN_CONFIRMATION_DEPTH
        self._mine_l1_and_wait_for_asm(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            btc_rpc=btc_rpc,
            mine_addr=mine_addr,
            blocks=boundary - btc_rpc.proxy.getblockcount(),
            timeout=PREDICATE_SETTLE_TIMEOUT_SECONDS,
        )

        # The old predicate authorizes coverage through the boundary. Hold L1
        # here until OL seals exactly that range; only accepting this checkpoint
        # promotes NeverAccept. A checkpoint straddling the boundary would be
        # rejected before exercising the new predicate.
        boundary_epoch = baseline["epoch"] + 1

        def find_boundary_checkpoint():
            nonlocal boundary_epoch
            info = strata_rpc.strata_getCheckpointInfo(boundary_epoch)
            if info is None:
                return None
            end_height = info["l1_range"][1]["height"]
            assert end_height <= boundary, (boundary, info)
            if end_height == boundary:
                return info
            boundary_epoch += 1
            return None

        boundary_checkpoint = wait_until_with_value(
            find_boundary_checkpoint,
            lambda info: info is not None,
            error_with=f"no checkpoint sealed at predicate boundary {boundary}",
            timeout=PREDICATE_SETTLE_TIMEOUT_SECONDS,
            step=1.0,
        )

        terminal = boundary_checkpoint["l2_end"]
        terminal_slot = terminal["slot"]
        terminal_block = strata_rpc.strata_getBlockBySlot(terminal_slot)
        assert terminal_block is not None, f"boundary terminal block {terminal_slot} is missing"
        assert terminal_block["header"]["blkid"] == terminal["blkid"], (
            terminal_block,
            boundary_checkpoint,
        )
        assert terminal_block["header"]["is_terminal"] is True, terminal_block
        self._assert_halted_at(strata, strata_rpc, terminal)

        logger.info("restarting the sequencer at predicate boundary slot %s", terminal_slot)
        strata.stop()
        time.sleep(RESTART_PAUSE_SECONDS)
        strata.start()
        strata_rpc = strata.wait_for_rpc_ready(timeout=RESTART_TIMEOUT_SECONDS)
        restored_tip = wait_until_with_value(
            lambda: strata.get_sync_status(strata_rpc)["tip"],
            lambda tip: tip["slot"] == terminal_slot and tip["blkid"] == terminal["blkid"],
            error_with=f"sequencer did not restore predicate boundary tip {terminal}",
            timeout=RESTART_TIMEOUT_SECONDS,
            step=0.5,
        )
        assert restored_tip["is_terminal"] is True, restored_tip
        self._assert_halted_at(strata, strata_rpc, terminal)
        logger.info("sequencer restart preserved the halt at slot %s", terminal_slot)

        mine_until_finalized_epoch(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            target_epoch=boundary_epoch,
            timeout=120,
            step=1.0,
        )
        assert self._finalized_epoch(strata, strata_rpc) == boundary_epoch
        finalized_checkpoint = strata_rpc.strata_getCheckpointInfo(boundary_epoch)
        assert finalized_checkpoint is not None
        assert finalized_checkpoint["confirmation_status"]["status"] == "finalized", (
            finalized_checkpoint
        )
        self._assert_halted_at(strata, strata_rpc, terminal)
        logger.info(
            "boundary checkpoint %s finalized at L1 height %s while OL tip remained at slot %s",
            boundary_epoch,
            boundary,
            terminal_slot,
        )

        # Keep advancing L1 after finalization. The node must continue following
        # ASM while refusing to construct any OL child of the boundary block.
        for _ in range(HALT_OBSERVATION_L1_BLOCKS):
            self._mine_l1_and_wait_for_asm(
                bitcoin=bitcoin,
                strata=strata,
                strata_rpc=strata_rpc,
                btc_rpc=btc_rpc,
                mine_addr=mine_addr,
                blocks=1,
                timeout=30,
            )
            self._assert_halted_at(strata, strata_rpc, terminal)

        next_epoch = boundary_epoch + 1
        assert strata_rpc.strata_getBlockBySlot(terminal_slot + 1) is None
        assert strata_rpc.strata_getCheckpointInfo(next_epoch) is None

        logger.info(
            "sequencer stayed halted at slot %s across %s additional L1 blocks",
            terminal_slot,
            HALT_OBSERVATION_L1_BLOCKS,
        )
        return True

    @staticmethod
    def _read_admin_xpriv(strata: StrataService) -> str:
        admin_key_path = Path(strata.props["datadir"]) / "bridge-operator_keys"
        if not admin_key_path.exists():
            raise AssertionError(f"admin key file not found: {admin_key_path}")
        admin_xpriv = admin_key_path.read_text().strip()
        if not admin_xpriv:
            raise AssertionError(f"admin key file is empty: {admin_key_path}")
        return admin_xpriv

    @staticmethod
    def _finalized_epoch(strata: StrataService, strata_rpc) -> int:
        return strata.get_sync_status(strata_rpc)["finalized"]["epoch"]

    @staticmethod
    def _mine_l1_and_wait_for_asm(
        bitcoin: BitcoinService,
        strata: StrataService,
        strata_rpc,
        btc_rpc,
        mine_addr,
        blocks: int,
        timeout: int,
    ) -> None:
        start_height = btc_rpc.proxy.getblockcount()
        btc_rpc.proxy.generatetoaddress(blocks, mine_addr)
        strata.wait_for_asm_manifest_commitment_at(
            start_height + blocks,
            rpc=strata_rpc,
            timeout=timeout,
            poll_interval=0.5,
        )

    @staticmethod
    def _assert_halted_at(strata: StrataService, strata_rpc, terminal: dict) -> None:
        tip = strata.get_sync_status(strata_rpc)["tip"]
        assert tip["slot"] == terminal["slot"], (tip, terminal)
        assert tip["blkid"] == terminal["blkid"], (tip, terminal)
        assert tip["is_terminal"] is True, tip
