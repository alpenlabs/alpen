import logging
import time

import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from utils import (
    get_envelope_pushdata,
    submit_da_blob,
    wait_until_with_value,
)
from utils.wait.strata import StrataWaiter


@flexitest.register
class IgnoreCheckpointWithInvalidProofTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.DualSequencerMixedPolicyEnvConfig(pre_generate_blocks=110))

    def main(self, ctx: flexitest.RunContext):
        """
        Test Scenario: Ensure nodes ignore L1 checkpoints with invalid proofs and
                       correctly process subsequent valid checkpoints for the same epoch.

        Test Strategy:
            - Run 2 sequencers with the same credentials but different configurations
              for proof timeout (strict vs. fastBatch).
            - Run 2 prover one for fastBatch and one for strict
            - Run 1 full node with a strict proof policy, requiring real proofs
              and rejecting empty/invalid ones.

        Test Steps:
            - Stop the strict prover, strict sequencer, and full node services.
            - Sequencer 1 (fastBatch) creates epoch 4 with an empty proof and publishes it to L1.
            - Stop sequencer 1.
            - Capture the L1 checkpoint DA envelope data for epoch 4 created by sequencer 1.
            - Stop the fast prover.
            - Start the strict prover, strict sequencer, and full node services.
            - Wait for sequencer 2 (strict) to process the initial epochs (up to 3).
            - Stop the strict prover temporarily.
            - Publish the captured L1 checkpoint DA envelope data (with the invalid proof)
              for epoch 4 to L1 again.
            - The full node and sequencer 2 should ignore the checkpoint with invalid proof,
              not finalize epoch 4 based on it, and wait for a valid checkpoint.
            - Start the strict prover again.
            - Both the full node and sequencer 2 should continue working normally,
              generate a valid proof, and finalize epoch 4 (and subsequently epoch 5)
              with the valid checkpoint.
            - After finalizing epoch 4 with the valid proof, resubmit the previous
              invalid proof data for epoch 4.
            - The full node and sequencer 2 should ignore this resubmission and continue
        """

        logging.warning(
            "Disabling test: Requires Taproot support in bitcoinlib and L1 syncing in the sequencer"
        )
        # WARNING:bitcoinlib.transactions:Taproot is not supported at the moment,
        # rest of parsing input transaction skipped
        return

        btc = ctx.get_service("bitcoin")
        seq_fast = ctx.get_service("seq_node_fast")
        seq_strict = ctx.get_service("seq_node_strict")
        prover_fast = ctx.get_service("prover_client_fast")
        prover_strict = ctx.get_service("prover_client_strict")
        fullnode = ctx.get_service("fullnode")
        seq_waiter = StrataWaiter(seq_fast.create_rpc(), self.logger, timeout=60)

        btcrpc: BitcoindClient = btc.create_rpc()
        seqrpc_fast = seq_fast.create_rpc()
        seqrpc_strict = seq_strict.create_rpc()

        prover_strict.stop()
        seq_strict.stop()
        fullnode.stop()

        # Wait for seq_fast to start
        seq_waiter.wait_until_client_ready()

        # Wait for the fast sequencer to create the first 3 epochs
        seq_waiter.wait_until_epoch_finalized(3)

        # Stop the fast prover so the next epoch's proof is not generated automatically
        prover_fast.stop()
        # Sleep briefly to allow the new epoch (epoch 4) to begin
        time.sleep(5)

        # Manually submit an empty proof for epoch 4 via the fast sequencer
        empty_proof_receipt = {"proof": [], "public_values": []}
        current_epoch = seqrpc_fast.strata_getLatestCheckpointIndex(None)
        logging.info(f"current_epoch on fast sequencer: {current_epoch}")  # Should be 4

        seqrpc_fast.strataadmin_submitCheckpointProof(current_epoch, empty_proof_receipt)

        # Wait for the fast sequencer to finalize epoch 4 using the empty proof
        seq_waiter.wait_until_epoch_finalized(4)

        # Get the commitment details for epoch 4 to find the L1 block hash containing the checkpoint
        epoch_4_commitment = wait_until_with_value(
            lambda: seqrpc_fast.strata_getEpochCommitments(4),
            predicate=lambda val: isinstance(val, list) and len(val) > 0,
            error_with="Epoch 4 commitment not found on fast sequencer",
            timeout=30,
        )
        logging.info(f"epoch_4_commitment from fast sequencer: {epoch_4_commitment}")
        # Extract the last slot from the first commitment in the list
        last_slot = epoch_4_commitment[0]["last_slot"]
        logging.info(
            f"epoch_4_last_slot from fast sequencer: {last_slot}"
        )  # Should be around 39 (10 * 4 - 1)

        # Find the L1 block status corresponding to the last slot of epoch 4
        verified_on = wait_until_with_value(
            lambda: seqrpc_fast.strata_getL2BlockStatus(last_slot),
            predicate=lambda val: isinstance(val, dict) and "Finalized" in val,
            error_with="L2 block status for slot {last_slot} not found or not finalized",
            timeout=30,
        )
        # Get the L1 block containing the checkpoint transaction with the empty proof
        verified_block_hash = btcrpc.proxy.getblockhash(verified_on["Finalized"])
        block_data = btcrpc.getblock(verified_block_hash)
        envelope_data = ""
        # Extract the DA envelope data from the transaction witness
        for tx in block_data["txs"]:
            try:
                envelope_data = get_envelope_pushdata(tx.witness_data().hex())
                logging.info("Found an envelope transaction in L1 block")
                break  # Assuming only one envelope per block in this test context
            except ValueError:
                continue
        else:
            raise Exception("Could not find envelope transaction in L1 block {verified_block_hash}")

        ## Stop fast sequencer related services
        # prover_fast is already stopped
        seq_fast.stop()

        ## Start strict sequencer related services
        prover_strict.start()
        seq_strict.start()
        fullnode.start()

        fullnode_rpc = fullnode.create_rpc()

        seq_strict_waiter = StrataWaiter(seqrpc_strict, self.logger, timeout=60)
        fn_waiter = StrataWaiter(fullnode_rpc, self.logger, timeout=60)

        ## Wait for strict sequencer to finalize epoch 3
        seq_strict_waiter.wait_until_epoch_finalized(3)

        # Stop the strict prover temporarily before submitting the invalid checkpoint
        prover_strict.stop()

        ## Check full node has also finalized epoch 3
        fn_waiter.wait_until_epoch_finalized(3)

        # Sleep briefly to allow the new epoch (epoch 4) to begin on the strict nodes
        time.sleep(5)

        # Submit the previously captured checkpoint (with invalid proof) for epoch 4 to L1
        tx_invalid_resubmit = submit_da_blob(btcrpc, seqrpc_strict, envelope_data)
        logging.info(
            f"Resubmitted checkpoint with invalid proof for epoch 4, tx: {tx_invalid_resubmit}"
        )

        # Allow time for the strict sequencer and full node to see the invalid checkpoint
        time.sleep(4)

        # Check that the strict sequencer and
        # full node have *not* finalized epoch 4 based on the invalid proof
        # Their last finalized epoch should still be 3.
        try:
            # Use a short timeout, as we expect these to fail
            seq_strict_waiter.wait_until_epoch_finalized(4)
            # If the above line didn't raise an exception,
            # it means epoch 4 was finalized incorrectly
            logging.warn("Strict sequencer incorrectly finalized epoch 4 with invalid proof")
            return False
        except Exception:
            logging.info("Strict sequencer correctly ignored invalid proof for epoch 4.")

        try:
            fn_waiter.wait_until_epoch_finalized(4)
            logging.warn("Full node incorrectly finalized epoch 4 with invalid proof")
            return False
        except Exception:
            logging.info("Full node correctly ignored invalid proof for epoch 4.")

        # Start the strict prover again to generate a valid proof for epoch 4
        prover_strict.start()

        # Wait for the strict sequencer to finalize epoch 5
        # (implying epoch 4 was finalized correctly)
        seq_strict_waiter.wait_until_epoch_finalized(5)

        # Check the full node also finalized epoch 5
        fn_waiter.wait_until_epoch_finalized(5)

        # Resubmit the invalid proof again after epoch 4 is properly finalized
        logging.info("Resubmitting the invalid proof for epoch 4 after it was finalized correctly")
        tx_invalid_after_valid = submit_da_blob(btcrpc, seqrpc_fast, envelope_data)
        logging.info(f"Resubmitted invalid proof again, tx: {tx_invalid_after_valid}")
        # Allow some time for nodes to potentially process it
        time.sleep(4)
        # Check that the last finalized epoch is still 5 (or higher if time passed)
        # This implicitly checks they ignored the resubmitted invalid proof.
        try:
            fn_waiter.wait_until_epoch_finalized(6)
        except Exception:
            logging.warn("Sequencer Failed after resubmitting the invalid proof for epoch 4")
            return False
        return True
