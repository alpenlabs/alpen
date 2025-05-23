import logging

import flexitest

from envs import testenv
from envs.testenv import BasicEnvConfig
from utils import *

# Test configuration for checkpoint-based prover
PROVER_CHECKPOINT_SETTINGS = {
    "CONSECUTIVE_PROOFS_REQUIRED": 3,
}


@flexitest.register
class ProverCheckpointEmptyProofRunnerTest(testenv.StrataTester):
    """This tests the epoch increments with empty proofs."""

    def __init__(self, ctx: flexitest.InitContext):
        rollup_settings = RollupParamsSettings.new_default()

        ctx.set_env(
            BasicEnvConfig(
                pre_generate_blocks=101,
                prover_client_settings=ProverClientSettings.new_default(),
                rollup_settings=rollup_settings,
            )
        )

    def main(self, ctx: flexitest.RunContext):
        sequencer = ctx.get_service("sequencer")
        sequencer_rpc = sequencer.create_rpc()

        empty_proof_receipt = {"proof": [], "public_values": []}

        current_epoch = sequencer_rpc.strata_getLatestCheckpointIndex(None)
        for _ in range(PROVER_CHECKPOINT_SETTINGS["CONSECUTIVE_PROOFS_REQUIRED"]):
            logging.info(f"Submitting proof for epoch {current_epoch}")

            # Submit empty proof
            sequencer_rpc.strataadmin_submitCheckpointProof(current_epoch, empty_proof_receipt)

            # Wait for epoch increment
            wait_until(
                lambda current_epoch=current_epoch: sequencer_rpc.strata_getLatestCheckpointIndex(
                    None
                )
                == current_epoch + 1,
                error_with="Checkpoint index did not increment",
            )

            current_epoch += 1
            logging.info(f"Epoch advanced to {current_epoch}")
