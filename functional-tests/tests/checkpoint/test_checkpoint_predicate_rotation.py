"""STR-3130: OL checkpoint predicate rotation is enforced end to end.

Predicate handovers are range-keyed. Enacting a transition at L1 height B
does not reject the next checkpoint outright: the outgoing predicate still
governs every checkpoint whose claimed L1 coverage ends at or below B, and
only coverage past B is verified against the incoming key. So a rotation to
`NeverAccept` drains — checkpoints still inside the old range keep
finalizing — and then stops permanently once coverage crosses B.

This test asserts both halves: that a checkpoint covering <= B is still
accepted *after* the rotation is enacted, and that the first checkpoint
covering > B never leaves `pending`.

The positive half needs a checkpoint that unambiguously belongs to the
post-enactment world, so the test manufactures one instead of hoping the
timing lines up: it mines to B, then holds L1 there while the OL seals
another epoch. That epoch did not exist when the rotation enacted, and L1
did not move while it sealed, so it is both freshly post-enactment and
inside the outgoing predicate's range.
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

# Budget for the OL to seal one epoch while L1 is held at the boundary. An
# epoch is `slots_per_epoch` OL blocks, so ~20s at the default 5s block time;
# this leaves room to have just missed a seal on arrival.
EPOCH_SEAL_TIMEOUT_SECONDS = 90

# Confirmation delay for the admin update. The transition is enacted at
# `confirm_height + depth`, which is also the handover boundary, so this sets
# the width of the window in which the outgoing predicate still governs. It
# must exceed the L1 span of a single epoch (see `DRAIN_STEP_SLEEP_SECONDS`)
# so that the drain has in-range checkpoints to work through rather than
# vaulting from below the boundary straight past it.
ADMIN_CONFIRMATION_DEPTH = 24

# Budget for draining the checkpoints still governed by the outgoing predicate
# and reaching the first one whose coverage crosses the boundary.
DRAIN_TIMEOUT_SECONDS = 240
DRAIN_L1_BLOCKS_PER_STEP = 1

# An epoch's L1 span is however many blocks are mined while it seals, and the
# OL seals on a wall-clock cadence regardless of L1. Pacing the drain keeps
# that span well inside `ADMIN_CONFIRMATION_DEPTH`; mining flat out makes each
# epoch cover tens of blocks and vault straight over the window.
DRAIN_STEP_SLEEP_SECONDS = 1.5

# Upstream logs this when the transition is enacted, carrying the boundary we
# derive independently. Used only as a cross-check.
ENACTMENT_LOG = "enacting checkpoint predicate transition"
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
        # nothing about the rotation. The finalized epoch here is the mark that
        # later finalizations are measured against.
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

        # Hold L1 at the boundary until the OL seals another epoch. That epoch
        # is the witness the positive half needs: it did not exist when the
        # rotation enacted, so nothing about it can have been accepted
        # beforehand, and L1 did not move while it sealed, so its coverage
        # cannot reach past the boundary. The enacted handover must accept it.
        witness_epoch = self._seal_epoch_at_boundary(strata_rpc)
        witness_info = self._wait_for_checkpoint_info(strata_rpc, witness_epoch)
        witness_coverage = self._coverage_end(witness_info)
        witness_status = self._checkpoint_status(witness_info)

        # Both guards cover the construction of the witness, not the protocol:
        # if its coverage reached past the boundary, or it had already been
        # accepted, it is not the under-the-boundary post-enactment checkpoint
        # the positive half needs.
        if witness_coverage is None or witness_coverage > boundary:
            raise AssertionError(
                f"witness epoch {witness_epoch} claims L1 coverage {witness_coverage}, not "
                f"<= boundary {boundary}, even though L1 was held at the boundary while it sealed"
            )
        if witness_status != "pending":
            raise AssertionError(
                f"witness epoch {witness_epoch} was already {witness_status!r} at the enactment "
                "height, so accepting it later would not say anything about the enacted handover"
            )
        logger.info(
            "epoch %s sealed with L1 held at boundary %s (coverage ends %s, still pending); "
            "the enacted handover must accept it",
            witness_epoch,
            boundary,
            witness_coverage,
        )

        blocked_epoch = self._drain_to_first_epoch_past_boundary(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            btc_rpc=btc_rpc,
            mine_addr=mine_addr,
            boundary=boundary,
        )
        plateau_epoch = blocked_epoch - 1

        # Positive half of the range-keyed semantics: the witness epoch — sealed
        # after the rotation had already enacted — must still have been accepted,
        # because its coverage stayed within the outgoing predicate's range.
        if plateau_epoch < witness_epoch:
            raise AssertionError(
                "no checkpoint sealed after the rotation was enacted finalized under the "
                f"outgoing predicate: epoch {witness_epoch} sealed with L1 held at boundary "
                f"{boundary}, but finalization only reached {plateau_epoch} before coverage "
                "crossed the boundary. The enacted handover should still accept checkpoints "
                "covering <= the boundary."
            )

        logger.info(
            "epochs %s..%s finalized under the outgoing predicate after enactment, "
            "including witness epoch %s (coverage ends %s <= %s)",
            finalized_at_enactment + 1,
            plateau_epoch,
            witness_epoch,
            witness_coverage,
            boundary,
        )

        self._assert_enactment_boundary(log_path, log_offset, boundary)

        # Negative half: the first checkpoint past the boundary must never move.
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
            finalized_epoch = self._finalized_epoch(strata, strata_rpc)
            if finalized_epoch > plateau_epoch:
                raise AssertionError(
                    "checkpoint finalized past the handover boundary under NeverAccept: "
                    f"plateau={plateau_epoch}, after={finalized_epoch}, boundary={boundary}"
                )

        checkpoint_info = strata_rpc.strata_getCheckpointInfo(blocked_epoch)
        checkpoint_status = self._checkpoint_status(checkpoint_info)
        if checkpoint_status != "pending":
            raise AssertionError(
                f"expected rejected checkpoint epoch {blocked_epoch} "
                f"(L1 coverage ends {self._coverage_end(checkpoint_info)} > boundary {boundary}) "
                f"to stay pending, got {checkpoint_status!r}"
            )

        logger.info(
            "checkpoint epoch %s stayed pending across %s L1 blocks after predicate rotation",
            blocked_epoch,
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

        Paced like the drain, and for the same reason: the epochs sealing in
        this window are the ones that must later finalize with coverage still
        inside the outgoing predicate's range, so their L1 spans have to stay
        narrow enough to land under `boundary`.
        """
        deadline = time.time() + ENACTMENT_TIMEOUT_SECONDS
        tip = btc_rpc.proxy.getblockcount()

        while tip < boundary:
            if time.time() >= deadline:
                raise AssertionError(
                    f"L1 did not reach the enactment height {boundary} within "
                    f"{ENACTMENT_TIMEOUT_SECONDS}s (tip {tip})"
                )
            self._mine_l1_and_wait_for_asm(
                bitcoin=bitcoin,
                strata=strata,
                strata_rpc=strata_rpc,
                btc_rpc=btc_rpc,
                mine_addr=mine_addr,
                blocks=DRAIN_L1_BLOCKS_PER_STEP,
                timeout=60,
            )
            time.sleep(DRAIN_STEP_SLEEP_SECONDS)
            tip = btc_rpc.proxy.getblockcount()

        # `_mine_l1_and_wait_for_asm` waited for the ASM to commit at the tip,
        # so the block that enacts the transition has been processed.
        return self._finalized_epoch(strata, strata_rpc)

    @staticmethod
    def _seal_epoch_at_boundary(strata_rpc) -> int:
        """Waits, without mining, for a fresh epoch to seal. Returns that epoch.

        `latest` is the most recently sealed epoch, so anything past the value
        read here sealed strictly after the rotation enacted — its checkpoint
        cannot have been posted, let alone accepted, beforehand. The OL seals on
        a wall-clock cadence regardless of L1, so parking L1 at the boundary
        also pins that epoch's L1 coverage at or below it.
        """
        already_sealed = int(strata_rpc.strata_getChainStatus()["latest"]["epoch"])
        return int(
            wait_until_with_value(
                lambda: int(strata_rpc.strata_getChainStatus()["latest"]["epoch"]),
                lambda epoch: epoch > already_sealed,
                error_with=(
                    f"OL sealed no epoch past {already_sealed} while L1 was held at the boundary"
                ),
                timeout=EPOCH_SEAL_TIMEOUT_SECONDS,
                step=0.5,
            )
        )

    def _drain_to_first_epoch_past_boundary(
        self,
        bitcoin: BitcoinService,
        strata: StrataService,
        strata_rpc,
        btc_rpc,
        mine_addr: str,
        boundary: int,
    ) -> int:
        """Mines until the first checkpoint whose coverage crosses `boundary`.

        Returns that epoch. Everything below it has finalized under the
        outgoing predicate, so the plateau is identified structurally — a
        checkpoint covering past the boundary can never be accepted — rather
        than by guessing that a run of quiet blocks means rejection.
        """
        deadline = time.time() + DRAIN_TIMEOUT_SECONDS
        last_seen = None

        while time.time() < deadline:
            self._mine_l1_and_wait_for_asm(
                bitcoin=bitcoin,
                strata=strata,
                strata_rpc=strata_rpc,
                btc_rpc=btc_rpc,
                mine_addr=mine_addr,
                blocks=DRAIN_L1_BLOCKS_PER_STEP,
                timeout=60,
            )
            time.sleep(DRAIN_STEP_SLEEP_SECONDS)

            finalized = self._finalized_epoch(strata, strata_rpc)
            candidate = finalized + 1
            info = strata_rpc.strata_getCheckpointInfo(candidate)
            if info is None:
                continue

            coverage_end = self._coverage_end(info)
            status = self._checkpoint_status(info)
            last_seen = (candidate, coverage_end, status)

            if coverage_end <= boundary:
                # Still governed by the outgoing predicate; let it finalize.
                continue

            if status != "pending":
                raise AssertionError(
                    f"checkpoint epoch {candidate} covers L1 up to {coverage_end}, past the "
                    f"handover boundary {boundary}, but is already {status!r} — the incoming "
                    "NeverAccept predicate is not governing its range"
                )

            logger.info(
                "epoch %s is the first checkpoint past the boundary "
                "(coverage ends %s > %s), status %s",
                candidate,
                coverage_end,
                boundary,
                status,
            )
            return candidate

        raise AssertionError(
            f"no checkpoint claimed L1 coverage past boundary {boundary} within "
            f"{DRAIN_TIMEOUT_SECONDS}s; last seen (epoch, coverage_end, status)={last_seen}. "
            "Either finalization is stuck on a checkpoint covering <= the boundary — which the "
            "enacted handover should still accept — or the sequencer stalled."
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
