import logging

import flexitest

from envs import testenv
from utils import *


@flexitest.register
class SyncGenesisTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(101))

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")

        # create both btc and sequencer RPC
        seqrpc = seq.create_rpc()
        seq_waiter = self.create_strata_waiter(seqrpc)

        seq_waiter.wait_for_genesis()

        # Make sure we're making progress.
        logging.info("observed genesis, checking that we're still making progress...")
        stat = None
        last_slot = 0
        for _ in range(5):
            stat = seq_waiter.wait_until_chain_tip_exceeds(last_slot, timeout=3)
            tip_slot = stat["tip_height"]
            tip_blkid = stat["tip_block_id"]
            cur_epoch = stat["cur_epoch"]
            logging.info(f"cur tip slot {tip_slot}, blkid {tip_blkid}, epoch {cur_epoch}")
            last_slot = tip_slot
