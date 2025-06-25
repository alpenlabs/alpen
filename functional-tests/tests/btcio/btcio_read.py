import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from utils import *


@flexitest.register
class L1StatusTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        rollup_params = RollupParamsSettings.new_default()
        rollup_params.horizon_height = 2
        rollup_params.genesis_trigger = 5
        ctx.set_env(
            testenv.BasicEnvConfig(0, rollup_settings=rollup_params, auto_generate_blocks=False)
        )

    def main(self, ctx: flexitest.RunContext):
        btc = ctx.get_service("bitcoin")
        seq = ctx.get_service("sequencer")
        # create both btc and sequencer RPC
        btcrpc: BitcoindClient = btc.create_rpc()
        seqrpc = seq.create_rpc()
        # generate 5 btc blocks
        generate_n_blocks(btcrpc, 5)

        seq_waiter = self.create_strata_waiter(seqrpc)

        # Wait for seq
        seq_waiter.wait_for_genesis(message="Timetout: waiting for genesis")

        received_block = btcrpc.getblock(btcrpc.proxy.getbestblockhash())
        l1stat = seq_waiter.wait_until_l1_height_at(received_block["height"])

        # Time is in millis
        cur_time = l1stat["last_update"] // 1000
        cur_l1_height = l1stat["cur_height"]

        # generate 2 more btc blocks
        generate_n_blocks(btcrpc, 2)

        next_l1stat = seq_waiter.wait_until_l1_height_at(cur_l1_height + 2)
        elapsed_time = next_l1stat["last_update"] // 1000

        # check if L1 reader is seeing new L1 activity
        assert next_l1stat["cur_height"] - l1stat["cur_height"] == 2, "new blocks not read"
        assert elapsed_time >= cur_time, "time not flowing properly"
