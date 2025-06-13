import logging

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
                lambda value, last_blk=last_blocknum: value > last_blk,
                error_with="Timeout: seem to not be making progress",
                timeout=3,
            )
            logging.info(f"current EL blocknum is {cur_blocknum}")
            last_blocknum = cur_blocknum
