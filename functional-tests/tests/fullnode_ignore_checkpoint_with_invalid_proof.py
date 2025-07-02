import logging

import flexitest

from envs import testenv

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

        seq_fast_rpc = seq_fast.create_rpc()
        seq_waiter = self.create_strata_waiter(seq_fast_rpc)

        # this fullnode has a strict proof policy but connected to the fast sequencer
        fullnode = ctx.get_service("fullnode")
        fn_waiter = self.create_strata_waiter(fullnode.create_rpc())

        prover_fast.stop()
        seq_strict.stop()
        prover_strict.stop()

        seq_waiter.wait_until_client_ready()
        fn_waiter.wait_until_client_ready()

        empty_proof_receipt = {"proof": [], "public_values": []}

        current_epoch = 0

        seq_waiter.wait_until_latest_checkpoint_at(current_epoch)

        for _ in range(PROVER_CHECKPOINT_SETTINGS["CONSECUTIVE_PROOFS_REQUIRED"]):
            logging.info(f"Submitting proof for epoch {current_epoch}")

            # Submit empty proof
            seq_fast_rpc.strataadmin_submitCheckpointProof(current_epoch, empty_proof_receipt)

            # Wait for epoch increment
            seq_waiter.wait_until_latest_checkpoint_at(current_epoch + 1)

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
