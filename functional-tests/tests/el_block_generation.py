import logging

import flexitest

from envs import testenv


@flexitest.register
class ElBlockGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(110))

    def main(self, ctx: flexitest.RunContext):
        seqrpc = ctx.get_service("sequencer").create_rpc()
        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()

        reth_waiter = self.create_reth_waiter(rethrpc)
        seq_waiter = self.create_strata_waiter(seqrpc)

        seq_waiter.wait_for_genesis()

        last_blocknum = int(rethrpc.eth_blockNumber(), 16)
        logging.info(f"initial EL blocknum is {last_blocknum}")

        for _ in range(5):
            cur_blocknum = reth_waiter.wait_until_eth_block_exceeds(last_blocknum)
            logging.info(f"current EL blocknum is {cur_blocknum}")
            last_blocknum = cur_blocknum
