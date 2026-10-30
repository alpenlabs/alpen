"""STR-4088: a fresh V1 network rotates to V2 on every test run.

Distinct native signing keys exercise per-epoch prover selection in normal CI.
The V2 program verifies every block of the first V2 epoch from its V1 start
state, so accepting its checkpoint also checks the switch at the first block.

Non-sequencer nodes currently use L1 checkpoint sync. A live follower and an
empty follower started after the upgrade independently reconstruct the chain;
block-executing fullnode coverage needs the future OL fullnode service profile.
"""

import logging
import re
import time
from pathlib import Path

import flexitest

from common.base_test import StrataNodeTest
from common.checkpoint_sync import check_summaries_equivalent, check_top_level_state_equivalent
from common.config import ServiceType
from common.config.constants import ALPEN_ACCOUNT_ID
from common.datatool import CHECKPOINT_TEST_PREDICATE_FILE, PREDICATE_FIXTURES_DIR
from common.services.strata import StrataService
from common.test_cli import create_checkpoint_predicate_update
from common.wait import wait_until_with_value
from envconfigs.checkpoint_sync import CheckpointSyncEnv
from tests.checkpoint.helpers import mine_until_finalized_epoch, tx_block_height

logger = logging.getLogger(__name__)
ROTATION_PREDICATE = (
    (PREDICATE_FIXTURES_DIR / "checkpoint-v2-bip340-schnorr-test.predicate").read_text().strip()
)
POST_ADMIN_UPDATE_L1_BLOCKS = 5
ENACTMENT_TIMEOUT_SECONDS = 180
EPOCH_SEAL_TIMEOUT_SECONDS = 120
L1_REORG_SAFE_DEPTH = 6
ADMIN_CONFIRMATION_DEPTH = 24
FINALIZATION_TIMEOUT_SECONDS = 240
V2_BLOCK_TIMEOUT_SECONDS = 60
PACE_L1_BLOCKS_PER_STEP = 1
PACE_STEP_SLEEP_SECONDS = 1.5
ENACTMENT_LOG = "recording checkpoint predicate transition"
ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;]*m")
BOUNDARY_FIELD_RE = re.compile(r"\bboundary=(\d+)")


@flexitest.register
class TestCheckpointPredicateRotation(StrataNodeTest):
    """Checks old-key catch-up, V2 finalization, restart, and cold L1 replay."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            CheckpointSyncEnv(
                pre_generate_blocks=110,
                seal_epoch_slots=8,
                ol_block_time_ms=1_000,
                fund_test_cli_wallet=True,
                admin_confirmation_depth=ADMIN_CONFIRMATION_DEPTH,
                l1_reorg_safe_depth=L1_REORG_SAFE_DEPTH,
                provision_recovery_node=True,
            )
        )

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)
        signer = self.get_service(ServiceType.StrataSigner)
        follower = self.get_service(ServiceType.StrataCheckpointNode)
        recovery = self.get_service(ServiceType.StrataRecoveryCheckpointNode)
        btc_rpc = bitcoin.create_rpc()
        rpc = strata.wait_for_rpc_ready(timeout=30)
        mine_addr = btc_rpc.proxy.getnewaddress()
        genesis_predicate = CHECKPOINT_TEST_PREDICATE_FILE.read_text().strip()
        assert genesis_predicate != ROTATION_PREDICATE, "rotation must change the key"
        assert rpc.strata_getSpecStatus()["cur_spec_version"] == 1

        baseline = mine_until_finalized_epoch(
            bitcoin, strata, rpc, target_epoch=1, timeout=FINALIZATION_TIMEOUT_SECONDS
        )
        result = create_checkpoint_predicate_update(
            seq_no=1,
            predicate=ROTATION_PREDICATE,
            admin_xpriv=self._read_admin_xpriv(strata),
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        log_path = Path(strata.props["datadir"]) / "service.log"
        log_offset = log_path.stat().st_size
        self._mine_l1_and_wait_for_asm(
            strata, rpc, btc_rpc, mine_addr, POST_ADMIN_UPDATE_L1_BLOCKS, 120
        )
        boundary = tx_block_height(btc_rpc, result["reveal_txid"]) + ADMIN_CONFIRMATION_DEPTH
        assert btc_rpc.proxy.getblockcount() < boundary

        # Seal an old-key checkpoint now, then mine empty blocks until B is
        # buried. This deterministically leaves a pre-B checkpoint in flight
        # when the admin update enacts, instead of relying on prover timing.
        in_flight_epoch = int(rpc.strata_getChainStatus()["latest"]["epoch"]) + 1
        in_flight = self._wait_for_checkpoint_info(rpc, in_flight_epoch)
        in_flight_end = self._coverage_end(in_flight)
        assert in_flight_end is not None and in_flight_end < boundary
        assert self._checkpoint_status(in_flight) == "pending"
        self._pace_l1_to(strata, rpc, btc_rpc, mine_addr, boundary, empty_blocks=True)
        assert rpc.strata_getSpecStatus()["cur_spec_version"] == 1
        self._assert_enactment_boundary(log_path, log_offset, boundary)
        self._pace_l1_to(
            strata,
            rpc,
            btc_rpc,
            mine_addr,
            boundary + L1_REORG_SAFE_DEPTH - 1,
            empty_blocks=True,
        )
        last_v1_epoch, last_v1_terminal = self._wait_for_epoch_ending_at(
            rpc, boundary, baseline["epoch"]
        )
        assert in_flight_epoch < last_v1_epoch
        assert self._checkpoint_status(rpc.strata_getCheckpointInfo(last_v1_epoch)) == "pending"
        assert self._checkpoint_status(rpc.strata_getCheckpointInfo(in_flight_epoch)) == "pending"

        # No further L1 block can accept a V2 checkpoint before this crash.
        # Kill the sequencer with the transition enacted but not settled;
        # restart must recover the boundary and pending old/new proof work.
        signer.stop()
        assert strata.proc is not None
        strata.proc.kill()
        strata.wait_for_down(timeout=30)
        strata.stop()
        strata.start()
        rpc = strata.wait_for_rpc_ready(timeout=60)
        signer.start()
        signer.wait_for_ready(timeout=20)
        recovered_epoch, recovered_terminal = self._wait_for_epoch_ending_at(
            rpc, boundary, baseline["epoch"]
        )
        assert (recovered_epoch, recovered_terminal) == (last_v1_epoch, last_v1_terminal)
        self._wait_for_v2_tip(rpc, last_v1_epoch)

        # Both the first V2 epoch (V1 start state) and a later V2 epoch
        # (V2 start state) must finalize. The ASM still needs the old key for
        # every in-flight V1 checkpoint, including the epoch ending at B.
        mine_until_finalized_epoch(
            bitcoin,
            strata,
            rpc,
            target_epoch=last_v1_epoch + 2,
            timeout=FINALIZATION_TIMEOUT_SECONDS,
        )
        for epoch in (in_flight_epoch, last_v1_epoch, last_v1_epoch + 1, last_v1_epoch + 2):
            info = rpc.strata_getCheckpointInfo(epoch)
            assert self._checkpoint_status(info) == "finalized", info
            assert info["confirmation_status"]["l1_reference"]["l1_block"]["height"] > boundary
        assert rpc.strata_getRejectedCheckpoints(0) == []

        # Stop signing and leave L1 fixed. Keep sequencer RPC available so
        # each follower comparison can read fresh finalized data.
        signer.stop()
        recovery.start()
        for node in (follower, recovery):
            node_rpc = node.wait_for_rpc_ready(timeout=60)
            sequencer_status, node_status = wait_until_with_value(
                lambda node=node: (strata.get_sync_status(rpc), node.get_sync_status()),
                lambda statuses: statuses[0]["finalized"] == statuses[1]["finalized"],
                error_with="checkpoint follower did not replay through the V2 upgrade",
                timeout=FINALIZATION_TIMEOUT_SECONDS,
            )
            check_top_level_state_equivalent(sequencer_status, node_status)
            spec_status = node_rpc.strata_getSpecStatus()
            assert spec_status["cur_spec_version"] == 2, spec_status
            assert spec_status["upgrade_required"] is None, spec_status
            target_epoch = int(sequencer_status["finalized"]["epoch"])
            for epoch in range(1, target_epoch + 1):
                expected = rpc.strata_getCheckpointInfo(epoch)
                actual = node_rpc.strata_getCheckpointInfo(epoch)
                # L1-only nodes do not retain the first block of an epoch.
                for field in ("idx", "l1_range", "l2_end", "confirmation_status"):
                    assert actual[field] == expected[field], (epoch, field, actual, expected)
                check_summaries_equivalent(
                    strata.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch, rpc=rpc),
                    node.get_account_epoch_summary(ALPEN_ACCOUNT_ID, epoch, rpc=node_rpc),
                )
        logger.info("V1 to V2 rotation replayed through epoch %s on both followers", target_epoch)
        return True

    def _pace_l1_to(
        self,
        strata: StrataService,
        strata_rpc,
        btc_rpc,
        mine_addr: str,
        height: int,
        empty_blocks: bool = False,
    ) -> None:
        """Mines up to L1 `height`, one block at a time, waiting for the ASM after each."""
        deadline = time.monotonic() + ENACTMENT_TIMEOUT_SECONDS
        tip = btc_rpc.proxy.getblockcount()

        while tip < height:
            if time.monotonic() >= deadline:
                raise AssertionError(
                    f"L1 did not reach height {height} within {ENACTMENT_TIMEOUT_SECONDS}s "
                    f"(tip {tip})"
                )
            self._mine_l1_and_wait_for_asm(
                strata=strata,
                strata_rpc=strata_rpc,
                btc_rpc=btc_rpc,
                mine_addr=mine_addr,
                blocks=PACE_L1_BLOCKS_PER_STEP,
                timeout=60,
                empty_blocks=empty_blocks,
            )
            time.sleep(PACE_STEP_SLEEP_SECONDS)
            tip = btc_rpc.proxy.getblockcount()

    def _wait_for_epoch_ending_at(
        self, strata_rpc, boundary: int, from_epoch: int
    ) -> tuple[int, str]:
        """Waits, without mining, for the OL to seal the epoch ending at `boundary`.

        Returns that epoch and its terminal block ID. It is the first epoch
        from `from_epoch` on whose checkpoint coverage reaches the boundary.
        The V2 epochs after it also end on the boundary while they process no
        manifests, so the latest epoch that does is not necessarily it.
        """

        def first_epoch_reaching_boundary():
            latest = int(strata_rpc.strata_getChainStatus()["latest"]["epoch"])
            for epoch in range(from_epoch, latest + 1):
                info = strata_rpc.strata_getCheckpointInfo(epoch)
                coverage_end = self._coverage_end(info)
                # Epochs get checkpoints in order, so a later epoch's
                # checkpoint cannot stand in for a missing earlier one.
                if coverage_end is None:
                    return None
                if coverage_end >= boundary:
                    return epoch, info
            return None

        epoch, info = wait_until_with_value(
            first_epoch_reaching_boundary,
            lambda value: value is not None,
            error_with=(
                f"OL sealed no epoch covering L1 up to the boundary {boundary} once the "
                "boundary was buried"
            ),
            timeout=EPOCH_SEAL_TIMEOUT_SECONDS,
            step=0.5,
        )
        coverage_end = self._coverage_end(info)
        if coverage_end != boundary:
            raise AssertionError(
                f"epoch {epoch} claims L1 coverage up to {coverage_end}, not the boundary "
                f"{boundary}: the OL did not seal on the enactment's L1 block"
            )
        return epoch, info["l2_end"]["blkid"]

    @staticmethod
    def _wait_for_v2_tip(strata_rpc, last_v1_epoch: int) -> None:
        """Waits for the node to build past `last_v1_epoch` under V2.

        That epoch processed the enactment, which ends V1, so the epoch after
        it runs V2. This binary implements V2, so the node keeps building and
        the spec status reports no upgrade.
        """
        status = wait_until_with_value(
            strata_rpc.strata_getSpecStatus,
            lambda status: status["cur_spec_version"] == 2,
            error_with=(
                f"the node built no V2 block after epoch {last_v1_epoch}, which ends at the "
                "enactment"
            ),
            timeout=V2_BLOCK_TIMEOUT_SECONDS,
            step=0.5,
        )
        if status["upgrade_required"] is not None:
            raise AssertionError(f"spec status reports an upgrade the node does not need: {status}")

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
        strata: StrataService,
        strata_rpc,
        btc_rpc,
        mine_addr,
        blocks: int,
        timeout: int,
        empty_blocks: bool = False,
    ) -> None:
        start_height = btc_rpc.proxy.getblockcount()
        if empty_blocks:
            for _ in range(blocks):
                btc_rpc.proxy.generateblock(mine_addr, [])
        else:
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
            timeout=EPOCH_SEAL_TIMEOUT_SECONDS,
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
