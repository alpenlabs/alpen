import logging

import flexitest

from envs import testenv
from utils import wait_until
from utils.wait.strata import StrataWaiter

PROVER_CHECKPOINT_SETTINGS = {
    "CONSECUTIVE_PROOFS_REQUIRED": 4,
}


@flexitest.register
class FullnodeIgnoreCheckpointWithInvalidProofTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.DualSequencerMixedPolicyEnvConfig(
                pre_generate_blocks=110, fullnode_is_strict_follower=False
            )
        )

    def main(self, ctx: flexitest.RunContext):
        """
        Test Scenario: Ensure fullnodes ignore L1 checkpoints with invalid proofs

        Test Strategy:
            - Run 1 sequencer with fastBatch proof policy
            - Run 1 full node with a strict proof policy, requiring real proofs
              and rejecting empty/invalid ones.
            - Fullnode should not finalize the epochs with empty proofs
        """

        seq_fast = ctx.get_service("seq_node_fast")
        prover_fast = ctx.get_service("prover_client_fast")
        seq_strict = ctx.get_service("seq_node_strict")
        prover_strict = ctx.get_service("prover_client_strict")
        seq_waiter = StrataWaiter(seq_fast.create_rpc(), self.logger)

        # this fullnode has a strict proof policy but connected to the fast sequencer
        fullnode = ctx.get_service("fullnode")
        fn_waiter = StrataWaiter(fullnode.create_rpc(), self.logger)

        prover_fast.stop()
        seq_strict.stop()
        prover_strict.stop()

        seq_fast_rpc = seq_fast.create_rpc()
        fullnode_rpc = fullnode.create_rpc()

        # Wait for seq_fast to start
        wait_until(
            lambda: seq_fast_rpc.strata_protocolVersion() is not None,
            error_with="Sequencer (fast) did not start on time",
        )

        # Wait for fullnode to start
        wait_until(
            lambda: fullnode_rpc.strata_protocolVersion() is not None,
            error_with="Fullnode did not start on time",
        )

        empty_proof_receipt = {"proof": [], "public_values": []}

        current_epoch = 0

        wait_until(
            lambda: seq_fast_rpc.strata_getLatestCheckpointIndex(None) == current_epoch,
            error_with="Checkpoint index did not increment",
        )

        for _ in range(PROVER_CHECKPOINT_SETTINGS["CONSECUTIVE_PROOFS_REQUIRED"]):
            logging.info(f"Submitting proof for epoch {current_epoch}")

            # Submit empty proof
            seq_fast_rpc.strataadmin_submitCheckpointProof(current_epoch, empty_proof_receipt)

            # Wait for epoch increment
            wait_until(
                lambda current_epoch=current_epoch: seq_fast_rpc.strata_getLatestCheckpointIndex(
                    None
                )
                == current_epoch + 1,
                error_with="Checkpoint index did not increment",
            )

            current_epoch += 1
            logging.info(f"Epoch advanced to {current_epoch}")

        logging.info("Waiting for epoch 3 to be finalized in the fast sequencer")
        seq_waiter.wait_until_epoch_finalized(3, timeout=20)

        try:
            logging.info("Checking if epoch 3 is finalized in the fullnode")
            fn_waiter.wait_until_epoch_finalized(3, timeout=20)
            self.warning("Fullnode incorrectly finalized epoch 3")
            return False
        except Exception:
            logging.info("Fullnode correctly ignored epochs because of the strict proof policy")

        return True
