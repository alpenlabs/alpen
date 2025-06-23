import logging

import flexitest

from envs import testenv
from utils import *
from utils.wait.strata import StrataWaiter


@flexitest.register
class SyncGenesisTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(101))

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")

        # create both btc and sequencer RPC
        seqrpc = seq.create_rpc()
        seq_waiter = StrataWaiter(seqrpc, self.logger, timeout=20, interval=2)

        seq_waiter.wait_for_genesis()

        # Make sure we're making progress.
        logging.info("observed genesis, checking that we're still making progress...")
        stat = None
        last_slot = 0
        for _ in range(5):
            stat = wait_until_with_value(
                lambda: seqrpc.strata_syncStatus(),
                lambda value, last_slt=last_slot: value["tip_height"] > last_slt,
                error_with="seem not to be making progress",
                timeout=3,
            )
            tip_slot = stat["tip_height"]
            tip_blkid = stat["tip_block_id"]
            cur_epoch = stat["cur_epoch"]
            logging.info(f"cur tip slot {tip_slot}, blkid {tip_blkid}, epoch {cur_epoch}")
            last_slot = tip_slot
