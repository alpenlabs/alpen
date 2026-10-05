"""STR-4084: a checkpoint mined on L1 but rejected by the ASM is reported.

Rotating the OL checkpoint predicate to `NeverAccept` makes the ASM reject every
checkpoint whose L1 coverage crosses the handover boundary, while the sequencer
keeps posting them and the broadcaster sees them confirm. The ASM emits nothing
for a rejection, so this is the case the submission tracker exists for.

With every service running, the test pushes L1 past the boundary and asserts
that once the first rejected checkpoint's block is buried at the reorg-safe
depth, `strata_getRejectedCheckpoints` reports it with the ASM's verified tip,
the node logs it exactly once, and `strata_getCheckpointInfo` still calls it
pending (detection changes no acceptance semantics).
"""

import logging
import re
from pathlib import Path

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.services.bitcoin import BitcoinService
from common.services.strata import StrataService
from common.test_cli import create_checkpoint_predicate_update
from envconfigs.strata import StrataEnvConfig
from tests.checkpoint.helpers import mine_until_finalized_epoch

logger = logging.getLogger(__name__)

# Pinned so the test knows how deep a reported rejection must be buried.
L1_REORG_SAFE_DEPTH = 4

# The rotation is enacted this many blocks after the admin update confirms, at
# the handover boundary. Small, since only checkpoints past it matter here.
ADMIN_CONFIRMATION_DEPTH = 8

# Budget for reaching the boundary, getting a checkpoint past it mined, and
# burying that checkpoint at the reorg-safe depth. Mining is paced so epochs
# keep sealing while L1 advances.
REJECTION_TIMEOUT_SECONDS = 360
MINE_STEP_SECONDS = 1.5

# Blocks mined after detection to check the report is not repeated.
EXTRA_L1_BLOCKS = 3

REJECTION_LOG = "checkpoint confirmed on L1 but rejected by ASM"
ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;]*m")
EPOCH_FIELD_RE = re.compile(r"\bepoch=(\d+)\b")


@flexitest.register
class TestCheckpointRejectionDetection(StrataNodeTest):
    """A checkpoint the ASM rejects is reported through RPC and the log."""

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

        mine_until_finalized_epoch(
            bitcoin=bitcoin,
            strata=strata,
            strata_rpc=strata_rpc,
            target_epoch=1,
            timeout=120,
            step=1.0,
        )
        reported = strata_rpc.strata_getRejectedCheckpoints()
        if reported != []:
            raise AssertionError(f"rejections reported under AlwaysAccept: {reported}")

        result = create_checkpoint_predicate_update(
            seq_no=1,
            predicate="NeverAccept",
            admin_xpriv=self._read_admin_xpriv(strata),
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        logger.info("submitted NeverAccept checkpoint predicate update: %s", result)

        log_path = Path(strata.props["datadir"]) / "service.log"
        log_offset = log_path.stat().st_size if log_path.exists() else 0

        reported = bitcoin.mine_until(
            check=strata_rpc.strata_getRejectedCheckpoints,
            predicate=lambda entries: len(entries) > 0,
            error_with="no rejected checkpoint reported after rotating to NeverAccept",
            timeout=REJECTION_TIMEOUT_SECONDS,
            step=MINE_STEP_SECONDS,
        )
        rejected = min(reported, key=lambda entry: entry["epoch"])
        epoch = rejected["epoch"]
        txid = rejected["txid"]
        logger.info("tracker reported rejected checkpoint: %s", rejected)

        # Every earlier epoch was accepted, and the ASM stopped right before this one.
        verified_tip = rejected["asm_verified_tip"]
        if verified_tip is None or verified_tip["epoch"] != epoch - 1:
            raise AssertionError(
                f"rejected epoch {epoch} reported with ASM verified tip {verified_tip}, "
                f"expected epoch {epoch - 1}"
            )

        # The reported transaction is mined where the tracker says, at the reorg-safe depth.
        tx = btc_rpc.proxy.getrawtransaction(txid, 1)
        block_height = int(btc_rpc.proxy.getblock(tx["blockhash"])["height"])
        if block_height != rejected["l1_block"]["height"]:
            raise AssertionError(
                f"tx {txid} is in block {block_height}, tracker reported {rejected['l1_block']}"
            )
        if tx["confirmations"] < L1_REORG_SAFE_DEPTH:
            raise AssertionError(
                f"rejection reported at {tx['confirmations']} confirmations, "
                f"below the reorg-safe depth {L1_REORG_SAFE_DEPTH}"
            )

        # It is a checkpoint the rotated predicate governs, which the ASM never accepted.
        reveal_height = self._tx_block_height(btc_rpc, result["reveal_txid"])
        boundary = reveal_height + ADMIN_CONFIRMATION_DEPTH
        info = strata_rpc.strata_getCheckpointInfo(epoch)
        coverage_end = int(info["l1_range"][1]["height"])
        if coverage_end <= boundary:
            raise AssertionError(
                f"rejected epoch {epoch} covers L1 up to {coverage_end}, "
                f"not past the handover boundary {boundary}"
            )
        status = info["confirmation_status"]["status"]
        if status != "pending":
            raise AssertionError(f"rejected epoch {epoch} has checkpoint status {status!r}")

        # Further observations neither duplicate the record nor log it again.
        for _ in range(EXTRA_L1_BLOCKS):
            start = btc_rpc.proxy.getblockcount()
            btc_rpc.proxy.generatetoaddress(1, btc_rpc.proxy.getnewaddress())
            strata.wait_for_asm_manifest_commitment_at(start + 1, rpc=strata_rpc, timeout=60)
        records = [
            entry for entry in strata_rpc.strata_getRejectedCheckpoints() if entry["txid"] == txid
        ]
        if records != [rejected]:
            raise AssertionError(f"expected one unchanged record for {txid}, got {records}")
        logged = self._rejection_log_lines(log_path, log_offset, txid)
        if len(logged) != 1:
            raise AssertionError(
                f"expected one {REJECTION_LOG!r} line for {txid}, got {len(logged)}: {logged}"
            )
        logged_epoch = EPOCH_FIELD_RE.search(logged[0])
        if logged_epoch is None or int(logged_epoch.group(1)) != epoch:
            raise AssertionError(f"rejection log line lacks epoch={epoch}: {logged[0]}")

        logger.info(
            "epoch %s rejected at L1 height %s (txid %s) reported once", epoch, block_height, txid
        )
        return True

    @staticmethod
    def _rejection_log_lines(log_path: Path, offset: int, txid: str) -> list[str]:
        with log_path.open("r", errors="replace") as handle:
            handle.seek(offset)
            # tracing writes ANSI colour codes, including between field names and values.
            plain = ANSI_ESCAPE_RE.sub("", handle.read())
        return [
            line for line in plain.splitlines() if REJECTION_LOG in line and f"txid={txid}" in line
        ]

    @staticmethod
    def _tx_block_height(btc_rpc, txid: str) -> int:
        blockhash = btc_rpc.proxy.getrawtransaction(txid, 1).get("blockhash")
        if not blockhash:
            raise AssertionError(f"tx {txid} is not confirmed yet, cannot derive its height")
        return int(btc_rpc.proxy.getblock(blockhash)["height"])

    @staticmethod
    def _read_admin_xpriv(strata: StrataService) -> str:
        admin_key_path = Path(strata.props["datadir"]) / "bridge-operator_keys"
        admin_xpriv = admin_key_path.read_text().strip()
        if not admin_xpriv:
            raise AssertionError(f"admin key file is empty: {admin_key_path}")
        return admin_xpriv
