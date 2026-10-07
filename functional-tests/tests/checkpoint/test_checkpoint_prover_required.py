"""Proof-requiring sequencers reject missing prover configuration; followers allow it."""

from pathlib import Path

import flexitest
import toml

from common.base_test import StrataNodeTest
from common.config import ServiceType
from envconfigs.checkpoint_sync import CheckpointSyncEnv


@flexitest.register
class TestCheckpointProverRequired(StrataNodeTest):
    """Omitting [prover] fails startup before the sequencer can publish empty proofs."""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(CheckpointSyncEnv(pre_generate_blocks=110))

    def main(self, ctx):
        sequencer = self.get_service(ServiceType.Strata)
        follower = self.get_service(ServiceType.StrataCheckpointNode)
        signer = self.get_service(ServiceType.StrataSigner)
        signer.stop()

        for node in (sequencer, follower):
            node.stop()
            config_path = Path(node.props["datadir"]) / "config.toml"
            config = toml.loads(config_path.read_text())
            del config["prover"]
            config_path.write_text(toml.dumps(config))

        # A follower verifies checkpoints but does not generate their proofs.
        follower.start()
        follower.wait_for_rpc_ready(timeout=30)

        log_path = Path(sequencer.props["datadir"]) / "service.log"
        log_offset = log_path.stat().st_size
        sequencer.start()
        sequencer.wait_for_down(timeout=30)
        assert sequencer.proc is not None
        assert sequencer.proc.returncode not in (None, 0), (
            "sequencer started without a required prover"
        )
        with log_path.open(errors="replace") as log:
            log.seek(log_offset)
            startup_log = log.read()
        assert "checkpoint prover configuration check failed at startup" in startup_log, startup_log
        assert "Active checkpoint predicate" in startup_log, startup_log
        assert "does not allow empty proofs" in startup_log, startup_log
        assert "requires AlwaysAccept checkpoint predicates" in startup_log, startup_log
        assert "health check server started" not in startup_log, startup_log
        return True
