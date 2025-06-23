import flexitest
from web3 import Web3

from envs import net_settings, testenv
from utils import *
from utils.wait.reth import RethWaiter
from utils.wait.strata import StrataWaiter


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
    print("txid", txid.to_0x_hex())

    web3.eth.wait_for_transaction_receipt(txid, timeout=5)


@flexitest.register
class ELSyncFromChainstateTest(testenv.StrataTestBase):
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
        reth = ctx.get_service("reth")
        web3: Web3 = reth.create_web3()

        seqrpc = seq.create_rpc()
        rethrpc = reth.create_rpc()

        block_wait_msg = "Timeout: waiting for reth blocks"
        reth_waiter = RethWaiter(rethrpc, timeout=10, message=block_wait_msg)
        seq_waiter = StrataWaiter(seqrpc, self.logger, timeout=20, interval=2)

        seq_waiter.wait_for_genesis()

        # workaround for issue restarting reth with no transactions
        for _ in range(3):
            send_tx(web3)

        wait_until_epoch_finalized(seqrpc, 0, timeout=30)

        # ensure there are some blocks generated
        reth_waiter.wait_until_eth_block_exceeds(0)

        self.info("stop sequencer")
        seq.stop()

        orig_blocknumber = get_latest_eth_block_number(rethrpc)
        print(f"stop reth @{orig_blocknumber}")
        reth.stop()

        # take snapshot of reth db
        SNAPSHOT_IDX = 1
        reth.snapshot_datadir(SNAPSHOT_IDX)

        self.info("start reth")
        reth.start()

        # wait for reth to start
        reth_waiter.wait_until_eth_block_exceeds(0)

        self.info("start sequencer")
        seq.start()

        # generate more blocks
        reth_waiter.wait_until_eth_block_exceeds(orig_blocknumber + 1)

        self.info("stop sequencer")
        seq.stop()
        final_blocknumber = get_latest_eth_block_number(rethrpc)

        self.info(f"stop reth @{final_blocknumber}")
        reth.stop()

        # replace reth db with older snapshot
        reth.restore_snapshot(SNAPSHOT_IDX)

        # sequencer now contains more blocks than in reth, should trigger EL sync later
        self.info("start reth")
        reth.start()

        # wait for reth to start
        reth_waiter.wait_until_eth_block_exceeds(0)

        # ensure reth db was reset to shorter chain
        assert get_latest_eth_block_number(rethrpc) < final_blocknumber

        self.info("start sequencer")
        seq.start()

        self.info("wait for sync")
        reth_waiter.wait_until_eth_block_exceeds(final_blocknumber)
