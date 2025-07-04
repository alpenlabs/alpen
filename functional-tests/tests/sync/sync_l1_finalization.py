import json
import logging

import flexitest

from envs import net_settings, testenv
from utils import *


@flexitest.register
class BlockFinalizationTest(testenv.StrataTestBase):
    """ """

    def __init__(self, ctx: flexitest.InitContext):
        premine_blocks = 101
        settings = net_settings.get_fast_batch_settings()
        settings.genesis_trigger = premine_blocks + 5

        ctx.set_env(
            testenv.BasicEnvConfig(
                premine_blocks,
                rollup_settings=settings,
                prover_client_settings=ProverClientSettings.new_with_proving(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")
        seqrpc = seq.create_rpc()

        prover = ctx.get_service("prover_client")
        prover_rpc = prover.create_rpc()

        num_epochs = 4

        seq_waiter = self.create_strata_waiter(seqrpc, timeout=60)
        epoch = seq_waiter.wait_until_chain_epoch(num_epochs)
        logging.info(f"epoch summary: {epoch}")

        cstat = seqrpc.strata_clientStatus()
        cstatdump = json.dumps(cstat, indent=2)
        logging.info(f"client status: {cstatdump}")

        # Wait for prover
        # TODO What is this check for?
        prover_waiter = self.create_prover_waiter(prover_rpc, timeout=30, interval=2)
        prover_waiter.wait_until_prover_ready()

        check_submit_proof_fails_for_nonexistent_batch(seqrpc, 100)

        # Wait until we get the checkpoint confirmed.
        wait_epoch_conf = 1
        seq_waiter.wait_until_epoch_confirmed(wait_epoch_conf, timeout=30)
        logging.info(f"Epoch {wait_epoch_conf} was confirmed!")

        # Wait until we get the expected number of epochs finalized.
        seq_waiter.wait_until_epoch_finalized(num_epochs, timeout=30)
        logging.info(f"Epoch {num_epochs} was finalized!")

        # FIXME what does this even check?
        # Check for first 4 checkpoints
        # for n in range(num_epochs):
        #    check_nth_checkpoint_finalized(n, seqrpc, prover_rpc)
        #    logging.info(f"Pass checkpoint finalization for checkpoint {n}")

        cstat = seqrpc.strata_clientStatus()
        cstatdump = json.dumps(cstat, indent=2)
        logging.info(f"client status: {cstatdump}")

        ss = seqrpc.strata_syncStatus()
        ssdump = json.dumps(ss, indent=2)
        logging.info(f"sync status: {ssdump}")

        seq_waiter.wait_until_epoch_observed_final(num_epochs)

        # Proof for checkpoint 0 is already sent above
        # FIXME do we still need this if we have the other checks?
        check_already_sent_proof(seqrpc, 0)
