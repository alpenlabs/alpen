import logging
import time
from dataclasses import dataclass

import flexitest

from envs import testenv
from envs.testenv import BasicEnvConfig
from utils import *


@dataclass
class ProverCheckpointSettings:
    """Test configuration for checkpoint-based prover"""

    consecutive_proofs_required: int = 3
    # waiting time for inner functions should end after 20 seconds, so 60 is more than enough
    prover_timeout_seconds: int = 60


@flexitest.register
class ProverCheckpointRunnerTest(testenv.StrataTestBase):
    """This tests the prover's checkpoint runner with an unreliable sequencer service.
    We check that a few (3) checkpoints are posted, restart the sequencer,
    and test that a few more (3) checkpoints get posted -- if so, the prover works properly.
    """

    def __init__(self, ctx: flexitest.InitContext):
        # Increase the proof timeout so that the checkpoint index increments only
        # after the prover client submits the corresponding checkpoint proof
        self.checkpoint_settings = ProverCheckpointSettings()

        rollup_settings = RollupParamsSettings.new_default()
        rollup_settings.proof_timeout = self.checkpoint_settings.prover_timeout_seconds

        ctx.set_env(
            BasicEnvConfig(
                pre_generate_blocks=101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=rollup_settings,
            )
        )

    def main(self, ctx: flexitest.RunContext):
        sequencer = ctx.get_service("sequencer")
        prover_client = ctx.get_service("prover_client")

        prover_rpc = prover_client.create_rpc()
        sequencer_rpc = sequencer.create_rpc()
        seq_waiter = self.create_strata_waiter(sequencer_rpc)

        # Wait until the prover client reports readiness
        wait_until(
            lambda: prover_rpc.dev_strata_getReport() is not None,
            error_with="Prover did not start on time",
        )

        # Wait for ASM to be ready
        seq_waiter.wait_until_asm_ready()

        epoch = seq_waiter.wait_until_next_chain_epoch()
        logging.info(f"it's now epoch {epoch}")
        epoch = seq_waiter.wait_until_next_chain_epoch()
        logging.info(f"it's now epoch {epoch}")

        # Wait for target epoch to be 'final'
        # Use syncStatus because it's quicker (than clientStatus)
        seq_waiter.wait_until_epoch_observed_final(
            self.checkpoint_settings.consecutive_proofs_required,
            timeout=self.checkpoint_settings.prover_timeout_seconds,
        )

        sequencer.stop()
        # Wait some time to make sure checkpoint runner indeed polls
        # on the checkpoint with the stopped (unreliable) sequencer.
        time.sleep(10)
        sequencer.start()
        sequencer_rpc = sequencer.create_rpc()
        seq_waiter.wait_until_client_ready(timeout=15)

        # Wait for target epoch to be confirmed; for debug use the same function as above,
        # it has more logs.
        # Use syncStatus because it's quicker (than clientStatus)
        seq_waiter.wait_until_chain_epoch(
            2 * self.checkpoint_settings.consecutive_proofs_required,
            timeout=self.checkpoint_settings.prover_timeout_seconds,
        )
