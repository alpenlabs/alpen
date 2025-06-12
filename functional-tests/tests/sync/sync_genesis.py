import logging
import time

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

        wait_for_genesis(seqrpc, timeout=20, step=2)

        # Make sure we're making progress.
        logging.info("observed genesis, checking that we're still making progress...")
        stat = None
        last_slot = 0
        for _ in range(5):
            stat = wait_until_with_value(
                lambda: seqrpc.strata_syncStatus(),
                lambda value: value["tip_height"] > last_slot,
                error_with="seem not to be making progress",
                timeout=3,
            )
            tip_slot = stat["tip_height"]
            tip_blkid = stat["tip_block_id"]
            cur_epoch = stat["cur_epoch"]
            logging.info(f"cur tip slot {tip_slot}, blkid {tip_blkid}, epoch {cur_epoch}")
            last_slot = tip_slot
