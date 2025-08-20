import logging

import flexitest
from web3 import Web3

from envs import net_settings, testenv
from utils import *


def send_tx(web3: Web3):
    dest = web3.to_checksum_address("deedf001900dca3ebeefdeadf001900dca3ebeef")
    txid = web3.eth.send_transaction(
        {
            "to": dest,
            "value": hex(1),
            "gas": hex(100000),
            "from": web3.address,
        }
    )
    web3.eth.wait_for_transaction_receipt(txid, timeout=5)


@flexitest.register
class CLReorgResumeBlockProductionTest(testenv.StrataTestBase):
    """This tests sync when el is missing blocks"""

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
        seq_signer = ctx.get_service("sequencer_signer")
        reth = ctx.get_service("reth")
        web3: Web3 = reth.create_web3()

        seqrpc = seq.create_rpc()
        rethrpc = reth.create_rpc()

        seq_waiter = self.create_strata_waiter(seqrpc)
        seq_waiter.wait_until_genesis()

        # workaround for issue restarting reth with no transactions
        for _ in range(3):
            send_tx(web3)

        seq_waiter.wait_until_epoch_finalized(0, timeout=30)

        # ensure there are some blocks generated
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > 0,
            error_with="not building blocks",
            timeout=5,
        )

        logging.info("stop sequencer")
        seq_signer.stop()
        orig_blocknumber = seqrpc.strata_syncStatus()["tip_height"]
        logging.info(f"stop seq @{orig_blocknumber}")
        seq.stop()

        reth.stop()

        # take snapshot of sequencer db
        SNAPSHOT_IDX = 1
        seq.snapshot_datadir(SNAPSHOT_IDX)

        logging.info("start reth")
        reth.start()

        # wait for reth to start
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > 0,
            error_with="reth did not start in time",
            timeout=5,
        )

        logging.info("start sequencer")
        seq.start()
        seq_signer.start()

        # generate more blocks
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > orig_blocknumber + 1,
            error_with="not building blocks",
            timeout=5,
        )

        logging.info("stop sequencer")
        seq_signer.stop()
        final_blocknumber = seqrpc.strata_syncStatus()["tip_height"]
        logging.info(f"stop reth @{final_blocknumber}")

        original_el_blockhash = rethrpc.eth_getBlockByNumber(hex(final_blocknumber), False)["hash"]

        seq.stop()

        # replace sequencer db with older snapshot
        seq.restore_snapshot(SNAPSHOT_IDX)

        logging.info("start sequencer")
        seq.start()
        # wait for reth to start
        wait_until(
            lambda: seqrpc.strata_syncStatus()["tip_height"] > 0,
            error_with="reth did not start in time",
            timeout=5,
        )
        # ensure sequencer db was reset to shorter chain
        assert seqrpc.strata_syncStatus()["tip_height"] < final_blocknumber

        seq_signer.start()

        logging.info("wait for block production to resume")
        wait_until(
            lambda: seqrpc.strata_syncStatus()["tip_height"] > final_blocknumber,
            error_with="not syncing blocks",
            timeout=10,
        )

        new_el_blockhash = rethrpc.eth_getBlockByNumber(hex(final_blocknumber), False)["hash"]

        assert original_el_blockhash != new_el_blockhash
