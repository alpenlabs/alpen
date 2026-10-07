"""STR-3130: OL checkpoint predicate rotation is enforced end to end.

Predicate handovers are range-keyed. Enacting a transition at L1 height B
does not reject the next checkpoint outright: the outgoing predicate still
governs every checkpoint whose claimed L1 coverage ends at or below B, and
only coverage past B is verified against the incoming key.

The enactment also ends the OL spec the node runs. The OL seals an epoch on
the enactment's L1 block, so that epoch's coverage ends exactly at B, and the
epoch after it runs the next spec. This binary implements no spec after V1,
so the node builds nothing past that epoch's terminal block, while it still
proves and posts that epoch's checkpoint.

This test asserts both halves: that the epoch ending at B, sealed after the
rotation enacted, is still accepted under the outgoing predicate and
finalizes, and that the node then stays at that epoch's terminal block, with
nothing finalizing past it, as L1 keeps advancing.
"""

import logging
import re
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

POST_ADMIN_UPDATE_L1_BLOCKS = 5
PREDICATE_REJECTION_L1_BLOCKS = 8
PREDICATE_SETTLE_TIMEOUT_SECONDS = 120

# Budget for pacing L1 from the reveal up to the enactment height.
ENACTMENT_TIMEOUT_SECONDS = 180

# Budget for the OL to seal the epoch that processes the enactment once the
# boundary is buried. The OL seals it on the block that processes the
# enactment's manifest, without waiting for the slot cadence.
EPOCH_SEAL_TIMEOUT_SECONDS = 90

# Block assembly only reads ASM manifests this many blocks deep, so the
# enactment's manifest at the boundary is processed once L1 is
# `L1_REORG_SAFE_DEPTH - 1` blocks past it. Set explicitly because the test
# mines exactly that far.
L1_REORG_SAFE_DEPTH = 6

# Confirmation delay for the admin update. The transition is enacted at
# `confirm_height + depth`, which is also the handover boundary.
ADMIN_CONFIRMATION_DEPTH = 24

# Budget for the epoch ending at the boundary to finalize once L1 moves on.
FINALIZATION_TIMEOUT_SECONDS = 240

PACE_L1_BLOCKS_PER_STEP = 1

# Pacing L1 keeps the OL in step with it, so the epoch that processes the
# enactment seals soon after L1 reaches the boundary.
PACE_STEP_SLEEP_SECONDS = 1.5

# Upstream logs this when the transition is enacted, carrying the boundary we
# derive independently. Used only as a cross-check.
ENACTMENT_LOG = "recording checkpoint predicate transition"
ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;]*m")
BOUNDARY_FIELD_RE = re.compile(r"\bboundary=(\d+)")


@flexitest.register
class TestCheckpointPredicateRotation(StrataNodeTest):
    """Rotating the OL checkpoint predicate changes ASM checkpoint acceptance."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=EpochSealingConfig(slots_per_epoch=4),
                fund_test_cli_wallet=True,
                admin_confirmation_depth=ADMIN_CONFIRMATION_DEPTH,
                l1_reorg_safe_depth=L1_REORG_SAFE_DEPTH,
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
        logger.info("baseline finalized epoch under AlwaysAccept: %s", baseline["epoch"])

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

        log_path = Path(strata.props["datadir"]) / "service.log"
        log_offset = log_path.stat().st_size if log_path.exists() else 0

        self._mine_l1_and_wait_for_asm(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            btc_rpc=btc_rpc,
            mine_addr=mine_addr,
            blocks=POST_ADMIN_UPDATE_L1_BLOCKS,
            timeout=PREDICATE_SETTLE_TIMEOUT_SECONDS,
        )

        # The transition is enacted `ADMIN_CONFIRMATION_DEPTH` blocks after the
        # reveal confirms, and the enactment height *is* the handover boundary.
        reveal_height = self._tx_block_height(btc_rpc, result["reveal_txid"])
        boundary = reveal_height + ADMIN_CONFIRMATION_DEPTH
        logger.info(
            "predicate update reveal confirmed at L1 height %s; handover boundary=%s",
            reveal_height,
            boundary,
        )

        # Advance L1 to the enactment height. Nothing before it can exercise
        # the handover: while the transition is still pending, `AlwaysAccept`
        # governs every checkpoint, so an epoch finalizing in that window says
        # nothing about the rotation.
        finalized_at_enactment = self._pace_l1_to_enactment(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            btc_rpc=btc_rpc,
            mine_addr=mine_addr,
            boundary=boundary,
        )
        logger.info(
            "rotation enacted at L1 height %s; finalized epoch there: %s",
            boundary,
            finalized_at_enactment,
        )

        # Bury the boundary so block assembly reads the enactment's manifest,
        # then wait, without mining, for the OL to seal the epoch that
        # processes it. Its coverage must end exactly at the boundary even
        # though L1 is past it: the OL seals on the enactment's L1 block.
        self._pace_l1_to(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            btc_rpc=btc_rpc,
            mine_addr=mine_addr,
            height=boundary + L1_REORG_SAFE_DEPTH - 1,
        )
        last_epoch, last_terminal = self._wait_for_epoch_ending_at(strata_rpc, boundary)
        last_info = self._wait_for_checkpoint_info(strata_rpc, last_epoch)
        last_status = self._checkpoint_status(last_info)

        # The epoch did not exist when the rotation enacted. It sealed only
        # after the last burying block, and nothing has been mined since, so
        # its checkpoint cannot have been accepted yet. Accepting it later is
        # what shows the enacted handover still applies the outgoing predicate
        # to coverage <= the boundary.
        if last_status != "pending":
            raise AssertionError(
                f"epoch {last_epoch}, which ends at the boundary {boundary}, was already "
                f"{last_status!r} at the enactment height, so accepting it later would not say "
                "anything about the enacted handover"
            )
        logger.info(
            "epoch %s sealed at the boundary %s with terminal block %s, still pending",
            last_epoch,
            boundary,
            last_terminal,
        )

        self._assert_enactment_boundary(log_path, log_offset, boundary)

        # Positive half of the range-keyed semantics: the epoch ending at the
        # boundary finalizes under the outgoing predicate.
        finalized = mine_until_finalized_epoch(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            target_epoch=last_epoch,
            timeout=FINALIZATION_TIMEOUT_SECONDS,
            step=1.0,
        )
        if finalized["epoch"] != last_epoch:
            raise AssertionError(
                f"finalized epoch {finalized['epoch']} is past the epoch {last_epoch} that "
                f"ends at the boundary {boundary}, which this node cannot build past"
            )
        logger.info("epoch %s finalized under the outgoing predicate", last_epoch)

        # Negative half: the epoch after the boundary runs a spec this binary
        # does not implement, so the node builds nothing past the terminal
        # block and nothing finalizes past it as L1 keeps advancing.
        for _ in range(PREDICATE_REJECTION_L1_BLOCKS):
            self._mine_l1_and_wait_for_asm(
                bitcoin=bitcoin,
                strata=strata,
                strata_rpc=strata_rpc,
                btc_rpc=btc_rpc,
                mine_addr=mine_addr,
                blocks=1,
                timeout=30,
            )
            time.sleep(PACE_STEP_SLEEP_SECONDS)
            self._assert_stopped_at(strata, strata_rpc, last_epoch, last_terminal)

        if strata_rpc.strata_getCheckpointInfo(last_epoch + 1) is not None:
            raise AssertionError(
                f"the node built a checkpoint for epoch {last_epoch + 1}, past the epoch that "
                f"ends at the boundary {boundary}"
            )

        logger.info(
            "the node stayed at the terminal block %s of epoch %s across %s L1 blocks",
            last_terminal,
            last_epoch,
            PREDICATE_REJECTION_L1_BLOCKS,
        )
        return True

    def _pace_l1_to_enactment(
        self,
        bitcoin: BitcoinService,
        strata: StrataService,
        strata_rpc,
        btc_rpc,
        mine_addr: str,
        boundary: int,
    ) -> int:
        """Mines up to the enactment height and returns the finalized epoch there.

        Paced so that the OL stays in step with L1 on the way to the boundary.
        """
        self._pace_l1_to(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            btc_rpc=btc_rpc,
            mine_addr=mine_addr,
            height=boundary,
        )

        # `_mine_l1_and_wait_for_asm` waited for the ASM to commit at the tip,
        # so the block that enacts the transition has been processed.
        return self._finalized_epoch(strata, strata_rpc)

    def _pace_l1_to(
        self,
        bitcoin: BitcoinService,
        strata: StrataService,
        strata_rpc,
        btc_rpc,
        mine_addr: str,
        height: int,
    ) -> None:
        """Mines up to L1 `height`, one block at a time, waiting for the ASM after each."""
        deadline = time.time() + ENACTMENT_TIMEOUT_SECONDS
        tip = btc_rpc.proxy.getblockcount()

        while tip < height:
            if time.time() >= deadline:
                raise AssertionError(
                    f"L1 did not reach height {height} within {ENACTMENT_TIMEOUT_SECONDS}s "
                    f"(tip {tip})"
                )
            self._mine_l1_and_wait_for_asm(
                bitcoin=bitcoin,
                strata=strata,
                strata_rpc=strata_rpc,
                btc_rpc=btc_rpc,
                mine_addr=mine_addr,
                blocks=PACE_L1_BLOCKS_PER_STEP,
                timeout=60,
            )
            time.sleep(PACE_STEP_SLEEP_SECONDS)
            tip = btc_rpc.proxy.getblockcount()

    def _wait_for_epoch_ending_at(self, strata_rpc, boundary: int) -> tuple[int, str]:
        """Waits, without mining, for the OL to seal the epoch ending at `boundary`.

        Returns that epoch and its terminal block ID. The epoch is the latest
        sealed one once its checkpoint coverage reaches the boundary, since
        the OL seals nothing after it.
        """

        def latest_epoch_coverage():
            latest = strata_rpc.strata_getChainStatus()["latest"]
            info = strata_rpc.strata_getCheckpointInfo(int(latest["epoch"]))
            return latest, self._coverage_end(info)

        latest, coverage_end = wait_until_with_value(
            latest_epoch_coverage,
            lambda value: value[1] is not None and value[1] >= boundary,
            error_with=(
                f"OL sealed no epoch covering L1 up to the boundary {boundary} once the "
                "boundary was buried"
            ),
            timeout=EPOCH_SEAL_TIMEOUT_SECONDS,
            step=0.5,
        )
        epoch = int(latest["epoch"])
        if coverage_end != boundary:
            raise AssertionError(
                f"epoch {epoch} claims L1 coverage up to {coverage_end}, not the boundary "
                f"{boundary}: the OL did not seal on the enactment's L1 block"
            )
        return epoch, latest["last_blkid"]

    def _assert_stopped_at(
        self, strata: StrataService, strata_rpc, epoch: int, terminal: str
    ) -> None:
        """Asserts that the node is still at `epoch`'s terminal block."""
        status = strata.get_sync_status(strata_rpc)
        tip = status["tip"]
        if tip["blkid"] != terminal or int(status["latest"]["epoch"]) != epoch:
            raise AssertionError(
                f"the node built past the terminal block {terminal} of epoch {epoch}, which "
                f"ends at the enactment: tip={tip}, latest={status['latest']}"
            )
        if int(status["finalized"]["epoch"]) != epoch:
            raise AssertionError(
                f"finalized epoch moved off {epoch}, the epoch ending at the enactment: "
                f"finalized={status['finalized']}"
            )

    @staticmethod
    def _tx_block_height(btc_rpc, txid: str) -> int:
        tx = btc_rpc.proxy.getrawtransaction(txid, 1)
        blockhash = tx.get("blockhash")
        if not blockhash:
            raise AssertionError(f"tx {txid} is not confirmed yet, cannot derive its height")
        return int(btc_rpc.proxy.getblock(blockhash)["height"])

    @staticmethod
    def _coverage_end(checkpoint_info: dict | None) -> int | None:
        """Last L1 height the checkpoint claims to have covered."""
        if checkpoint_info is None:
            return None
        return int(checkpoint_info["l1_range"][1]["height"])

    @staticmethod
    def _assert_enactment_boundary(log_path: Path, offset: int, boundary: int) -> None:
        """Cross-checks the derived boundary against ASM's own enactment log.

        The arithmetic in `main` is the authority; this catches drift if
        upstream changes when a transition is enacted. Skipped only when the
        log line is absent entirely (a quieter RUST_LOG than CI and
        `run_tests.sh` use) — if the line is there but unparsable, that is a
        format change worth failing on rather than silently ignoring.
        """
        if not log_path.exists():
            return
        with log_path.open("r", errors="replace") as handle:
            handle.seek(offset)
            tail = handle.read()

        # tracing writes ANSI colour codes, including between the field name
        # and its value, so strip them before matching.
        plain = ANSI_ESCAPE_RE.sub("", tail)

        for line in plain.splitlines():
            if ENACTMENT_LOG not in line:
                continue
            match = BOUNDARY_FIELD_RE.search(line)
            if match is None:
                raise AssertionError(
                    f"found {ENACTMENT_LOG!r} in the node log but no `boundary=` field: {line!r}"
                )
            logged = int(match.group(1))
            if logged != boundary:
                raise AssertionError(
                    f"derived handover boundary {boundary} disagrees with ASM's "
                    f"enactment log ({logged}); the enactment height rule changed"
                )
            logger.info("enactment log confirms boundary=%s", logged)
            return

        logger.warning(
            "enactment log line %r not found; boundary %s not cross-checked",
            ENACTMENT_LOG,
            boundary,
        )

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
    def _wait_for_checkpoint_info(strata_rpc, epoch: int) -> dict:
        return wait_until_with_value(
            lambda: strata_rpc.strata_getCheckpointInfo(epoch),
            lambda info: info is not None,
            error_with=f"checkpoint info for epoch {epoch} was not created",
            timeout=120,
            step=1.0,
        )

    @staticmethod
    def _checkpoint_status(checkpoint_info: dict | None) -> str | None:
        if checkpoint_info is None:
            return None

        status = checkpoint_info.get("confirmation_status")
        if isinstance(status, str):
            return status.lower()
        if isinstance(status, dict):
            return status.get("status")
        return None
