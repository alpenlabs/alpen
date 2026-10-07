"""Missing pending artifacts allow supported proofs but reject a subsequent startup."""

import re
from pathlib import Path

import flexitest

from common.base_test import StrataNodeTest
from common.config import EpochSealingConfig, ServiceType
from common.test_cli import create_checkpoint_predicate_update
from common.wait import wait_until, wait_until_with_value
from envconfigs.strata import StrataEnvConfig
from tests.checkpoint.helpers import mine_until_finalized_epoch

ADMIN_CONFIRMATION_DEPTH = 24
ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;]*m")
EPOCH_FIELD_RE = re.compile(r"\bepoch=(\d+)")


@flexitest.register
class TestCheckpointArtifactAvailability(StrataNodeTest):
    """Supported epochs keep proving, but startup requires all active and pending artifacts."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            StrataEnvConfig(
                pre_generate_blocks=110,
                epoch_sealing=EpochSealingConfig(slots_per_epoch=4),
                ol_block_time_ms=1_000,
                fund_test_cli_wallet=True,
                admin_confirmation_depth=ADMIN_CONFIRMATION_DEPTH,
            )
        )

    def main(self, ctx):
        bitcoin = self.get_service(ServiceType.Bitcoin)
        strata = self.get_service(ServiceType.Strata)
        signer = self.get_service(ServiceType.StrataSigner)
        btc_rpc = bitcoin.create_rpc()
        strata_rpc = strata.wait_for_rpc_ready(timeout=20)
        mine_addr = btc_rpc.proxy.getnewaddress()
        mine_until_finalized_epoch(bitcoin, strata, strata_rpc, target_epoch=1)

        log_path = Path(strata.props["datadir"]) / "service.log"
        log_offset = log_path.stat().st_size
        admin_xpriv = (Path(strata.props["datadir"]) / "bridge-operator_keys").read_text().strip()
        update = create_checkpoint_predicate_update(
            seq_no=1,
            predicate="NeverAccept",
            admin_xpriv=admin_xpriv,
            btc_url=bitcoin.props["rpc_url"],
            btc_user=bitcoin.props["rpc_user"],
            btc_password=bitcoin.props["rpc_password"],
        )
        btc_rpc.proxy.generatetoaddress(5, mine_addr)
        reveal = btc_rpc.proxy.getrawtransaction(update["reveal_txid"], 1)
        reveal_height = int(btc_rpc.proxy.getblock(reveal["blockhash"])["height"])
        boundary = reveal_height + ADMIN_CONFIRMATION_DEPTH
        strata.wait_for_asm_manifest_commitment_at(
            btc_rpc.proxy.getblockcount(), rpc=strata_rpc, timeout=60
        )
        self._wait_for_new_proof(strata_rpc, log_path, log_offset, boundary)

        btc_rpc.proxy.generatetoaddress(boundary - btc_rpc.proxy.getblockcount(), mine_addr)
        strata.wait_for_asm_manifest_commitment_at(boundary, rpc=strata_rpc, timeout=60)
        # The manifest is written before the anchor. Wait for the completed state
        # commit so the restart below must observe the enacted pending predicate.
        self._wait_for_asm_state_commit(log_path, log_offset, boundary)

        # Hold L1 at the enactment boundary: old V1 epochs can still be proven,
        # while no new L1 block can accept a checkpoint that promotes the pending predicate.
        self._wait_for_new_proof(strata_rpc, log_path, log_offset, boundary)

        signer.stop()
        strata.stop()
        log_offset = log_path.stat().st_size
        strata.start()
        strata.wait_for_down(timeout=30)
        assert strata.proc is not None
        assert strata.proc.returncode not in (None, 0), (
            "missing required artifacts did not fail startup"
        )
        with log_path.open(errors="replace") as log:
            log.seek(log_offset)
            startup_log = ANSI_ESCAPE_RE.sub("", log.read())
        assert "checkpoint artifact check failed at startup" in startup_log, startup_log
        assert "required checkpoint artifacts are missing:" in startup_log, startup_log
        assert "MissingCheckpointArtifact" in startup_log and "Pending" in startup_log, startup_log
        assert "checkpoint prover services started" not in startup_log, startup_log
        assert btc_rpc.proxy.getblockcount() == boundary
        return True

    @staticmethod
    def _wait_for_asm_state_commit(log_path: Path, offset: int, height: int):
        def state_is_committed():
            with log_path.open(errors="replace") as log:
                log.seek(offset)
                lines = ANSI_ESCAPE_RE.sub("", log.read()).splitlines()
            return any(
                "ASM transition complete, manifest and state stored" in line
                and f"block_id={height}@" in line
                for line in lines
            )

        wait_until(
            state_is_committed,
            error_with=f"ASM state at L1 height {height} was not committed",
            timeout=30,
            step=0.5,
        )

    @staticmethod
    def _wait_for_new_proof(strata_rpc, log_path: Path, offset: int, boundary: int):
        # RPC checkpoint info is available from the epoch summary before its proof.
        # The completion log is emitted after proof persistence and canonicality
        # validation; unlike a signing duty it remains observable after signing.
        target_epoch = int(strata_rpc.strata_getChainStatus()["latest"]["epoch"]) + 1

        def completed_epoch():
            with log_path.open(errors="replace") as log:
                log.seek(offset)
                lines = ANSI_ESCAPE_RE.sub("", log.read()).splitlines()
            for line in lines:
                if "checkpoint proof completed" not in line:
                    continue
                match = EPOCH_FIELD_RE.search(line)
                if match and int(match.group(1)) >= target_epoch:
                    return int(match.group(1))
            return None

        epoch = wait_until_with_value(
            completed_epoch,
            lambda value: value is not None,
            error_with=f"no new proof completed from epoch {target_epoch}",
            timeout=90,
            step=0.5,
        )
        checkpoint = strata_rpc.strata_getCheckpointInfo(epoch)
        assert checkpoint is not None
        assert int(checkpoint["l1_range"][1]["height"]) <= boundary, (
            f"epoch {epoch} crossed the pending transition at L1 {boundary}"
        )
