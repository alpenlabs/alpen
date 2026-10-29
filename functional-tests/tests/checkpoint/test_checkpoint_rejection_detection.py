"""STR-4084: a checkpoint mined on L1 but rejected by the ASM is reported.

The test asks the sequencer, through the `debug_corruptNextCheckpointProof`
debug RPC, to flip a byte of the next checkpoint proof it builds. The ASM
rejects that checkpoint under the active predicate, while the sequencer keeps
posting checkpoints and the broadcaster sees them confirm. The ASM only logs a
tracing warning for a rejection and emits no ASM log, so this is the case the
submission tracker exists for.

A checkpoint predicate rotation cannot produce this case. Its enactment ends
the spec this binary runs, so the node builds nothing past the handover
boundary and never posts a checkpoint the incoming predicate governs.

With every service running, the test pushes L1 on and asserts that once the
corrupted checkpoint's block is buried at the reorg-safe depth,
`strata_getRejectedCheckpoints` reports it with the ASM's verified tip, the
node logs it exactly once, and `strata_getCheckpointInfo` still calls it
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
from envconfigs.strata import StrataEnvConfig
from tests.checkpoint.helpers import mine_until_finalized_epoch, strip_ansi

logger = logging.getLogger(__name__)

# Pinned so the test knows how deep a reported rejection must be buried.
L1_REORG_SAFE_DEPTH = 4

# Budget for building and posting the corrupted checkpoint and burying it at
# the reorg-safe depth. Mining is paced so epochs keep sealing while L1 advances.
REJECTION_TIMEOUT_SECONDS = 360
MINE_STEP_SECONDS = 1.5

# Blocks mined after detection to check the report is not repeated.
EXTRA_L1_BLOCKS = 3

CORRUPTION_LOG = "corrupted checkpoint proof on request"
REJECTION_LOG = "checkpoint confirmed on L1 but rejected by ASM"
EPOCH_FIELD_RE = re.compile(r"\bepoch=(\d+)\b")


@flexitest.register
class TestCheckpointRejectionDetection(StrataNodeTest):
    """A checkpoint the ASM rejects is reported through RPC and the log."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=EpochSealingConfig(slots_per_epoch=4),
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
        reported = strata_rpc.strata_getRejectedCheckpoints(0)
        if reported != []:
            raise AssertionError(f"rejections reported before any proof was corrupted: {reported}")

        log_path = Path(strata.props["datadir"]) / "service.log"
        log_offset = log_path.stat().st_size if log_path.exists() else 0

        strata_rpc.debug_corruptNextCheckpointProof()
        logger.info("requested a corrupted proof for the next checkpoint")

        reported = bitcoin.mine_until(
            check=lambda: strata_rpc.strata_getRejectedCheckpoints(0),
            predicate=lambda entries: len(entries) > 0,
            error_with="no rejected checkpoint reported after corrupting a proof",
            timeout=REJECTION_TIMEOUT_SECONDS,
            step=MINE_STEP_SECONDS,
        )
        rejected = min(reported, key=lambda entry: entry["epoch"])
        epoch = rejected["epoch"]
        txid = rejected["txid"]
        logger.info("tracker reported rejected checkpoint: %s", rejected)

        # It is the checkpoint whose proof was corrupted, the only one.
        corrupted = self._corrupted_epochs(log_path, log_offset)
        if corrupted != [epoch]:
            raise AssertionError(
                f"rejected epoch {epoch} reported, but proofs were corrupted for epochs {corrupted}"
            )

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

        # The ASM never accepted it.
        info = strata_rpc.strata_getCheckpointInfo(epoch)
        status = info["confirmation_status"]["status"]
        if status != "pending":
            raise AssertionError(f"rejected epoch {epoch} has checkpoint status {status!r}")

        # Further observations neither duplicate the record nor log it again.
        for _ in range(EXTRA_L1_BLOCKS):
            start = btc_rpc.proxy.getblockcount()
            btc_rpc.proxy.generatetoaddress(1, btc_rpc.proxy.getnewaddress())
            strata.wait_for_asm_manifest_commitment_at(start + 1, rpc=strata_rpc, timeout=60)
        records = [
            entry
            for entry in strata_rpc.strata_getRejectedCheckpoints(epoch)
            if entry["txid"] == txid
        ]
        if records != [rejected]:
            raise AssertionError(f"expected one unchanged record for {txid}, got {records}")
        logged = [
            line
            for line in self._log_lines_since(log_path, log_offset)
            if REJECTION_LOG in line and f"txid={txid}" in line
        ]
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

    @classmethod
    def _corrupted_epochs(cls, log_path: Path, offset: int) -> list[int]:
        """Epochs the sequencer logged a corrupted proof for since `offset`."""
        epochs = []
        for line in cls._log_lines_since(log_path, offset):
            if CORRUPTION_LOG not in line:
                continue
            match = EPOCH_FIELD_RE.search(line)
            if match is None:
                raise AssertionError(f"proof corruption log line lacks an epoch: {line}")
            epochs.append(int(match.group(1)))
        return epochs

    @staticmethod
    def _log_lines_since(log_path: Path, offset: int) -> list[str]:
        with log_path.open("r", errors="replace") as handle:
            handle.seek(offset)
            plain = strip_ansi(handle.read())
        return plain.splitlines()
