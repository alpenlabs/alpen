import logging
import time

import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from utils import *
from utils.constants import MAX_HORIZON_POLL_INTERVAL_SECS


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

        # Wait for seq
        wait_for_genesis(seqrpc, timeout=30)

        time.sleep(3)

        received_block = btcrpc.getblock(btcrpc.proxy.getbestblockhash())
        l1stat = seqrpc.strata_l1status()

        # Time is in millis
        cur_time = l1stat["last_update"] // 1000

        # check if height on bitcoin is same as, it is seen in sequencer
        logging.info(f"L1 stat curr height: {l1stat['cur_height']}")
        logging.info(f"Received from bitcoin: {received_block['height']}")
        seq_height = l1stat["cur_height"]
        block_height = received_block["height"]
        assert seq_height == block_height, (
            f"sequencer height {seq_height} doesn't match the bitcoin node height {block_height}"
        )

        # generate 2 more btc blocks
        generate_n_blocks(btcrpc, 2)
        time.sleep(MAX_HORIZON_POLL_INTERVAL_SECS * 2)

        next_l1stat = seqrpc.strata_l1status()
        elapsed_time = next_l1stat["last_update"] // 1000

        # check if L1 reader is seeing new L1 activity
        assert next_l1stat["cur_height"] - l1stat["cur_height"] == 2, "new blocks not read"
        assert elapsed_time > cur_time, "time not flowing properly"
