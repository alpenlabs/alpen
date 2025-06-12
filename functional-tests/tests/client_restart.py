import flexitest

from envs import net_settings, testenv
from utils import *


@flexitest.register
class BlockFinalizationSeqRestartTest(testenv.StrataTestBase):
    """This tests finalization when sequencer client restarts"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")
        seqrpc = seq.create_rpc()

        prover = ctx.get_service("prover_client")
        prover_rpc = prover.create_rpc()

        wait_for_genesis(seqrpc, timeout=10, step=2)

        # Wait for prover
        wait_until(
            lambda: prover_rpc.dev_strata_getReport() is not None,
            error_with="Prover did not start on time",
        )

        check_submit_proof_fails_for_nonexistent_batch(seqrpc, 100)

        # Check for first 2 checkpoints.  I don't know why this takes so long to
        # get started, but once it does it goes fairly quickly.
        for n in range(2):
            check_nth_checkpoint_finalized(n, seqrpc, prover_rpc, timeout=150)
            logging.info(f"Found checkpoint {n} finalized")

        # Restart sequencer.
        logging.info("Restarting sequencer's node...")
        seq.stop()
        seq.start()
        logging.info("Waiting for it to come back up...")
        seqrpc = seq.create_rpc()
        wait_until(seqrpc.strata_protocolVersion, timeout=5)

        # Check for next 2 checkpoints
        logging.info("Now we look for more checkpoints")
        for n in range(2, 4):
            check_nth_checkpoint_finalized(n, seqrpc, prover_rpc, timeout=150)
            logging.info(f"Found checkpoint {n} finalized")

        check_already_sent_proof(seqrpc, 0)
