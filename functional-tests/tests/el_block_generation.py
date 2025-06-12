import logging
import time

import flexitest

from envs import testenv
from utils import wait_for_genesis
from utils.utils import wait_until_with_value


@flexitest.register
class ElBlockGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(110))

    def main(self, ctx: flexitest.RunContext):
        seqrpc = ctx.get_service("sequencer").create_rpc()
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()

        wait_for_genesis(seqrpc, timeout=20)

        last_blocknum = int(rethrpc.eth_blockNumber(), 16)
        logging.info(f"initial EL blocknum is {last_blocknum}")

        for _ in range(5):
            cur_blocknum = wait_until_with_value(
                lambda: int(rethrpc.eth_blockNumber(), 16),
                lambda value: value >= last_blocknum,
                error_with="Timeout: Current block seems to go backwards",
                timeout=3,
            )
            logging.info(f"current EL blocknum is {cur_blocknum}")
            assert cur_blocknum > last_blocknum, "seem to not be making progress"
            last_blocknum = cur_blocknum
