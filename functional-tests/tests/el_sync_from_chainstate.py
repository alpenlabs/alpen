import flexitest
from web3 import Web3

from envs import testenv
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
    print("txid", txid.to_0x_hex())

    web3.eth.wait_for_transaction_receipt(txid, timeout=5)


@flexitest.register
class ELSyncFromChainstateTest(testenv.StrataTester):
    """This tests sync when el is missing blocks"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(testenv.BasicEnvConfig(101))

    def main(self, ctx: flexitest.RunContext):
        try:
            self.run(ctx)
        finally:
            seq = ctx.get_service("sequencer")
            reth = ctx.get_service("reth")

            if not seq.is_started():
                seq.start()
            if not reth.is_started():
                reth.start()

    def run(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")
        reth = ctx.get_service("reth")
        web3: Web3 = reth.create_web3()

        seqrpc = seq.create_rpc()
        rethrpc = reth.create_rpc()

        wait_for_genesis(seqrpc, timeout=20)

        # workaround for issue restarting reth with no transactions
        for _ in range(3):
            send_tx(web3)

        # ensure there are some blocks generated
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > 0,
            error_with="not building blocks",
            timeout=5,
        )

        print("stop sequencer")
        seq.stop()

        orig_blocknumber = int(rethrpc.eth_blockNumber(), base=16)
        print(f"stop reth @{orig_blocknumber}")
        reth.stop()

        # take snapshot of reth db
        SNAPSHOT_IDX = 1
        reth.snapshot_datadir(SNAPSHOT_IDX)

        print("start reth")
        reth.start()

        # wait for reth to start
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > 0,
            error_with="reth did not start in time",
            timeout=5,
        )

        print("start sequencer")
        seq.start()

        # generate more blocks
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > orig_blocknumber + 1,
            error_with="not building blocks",
            timeout=5,
        )

        print("stop sequencer")
        seq.stop()
        final_blocknumber = int(rethrpc.eth_blockNumber(), base=16)

        print(f"stop reth @{final_blocknumber}")
        reth.stop()

        # replace reth db with older snapshot
        reth.restore_snapshot(SNAPSHOT_IDX)

        # sequencer now contains more blocks than in reth, should trigger EL sync later
        print("start reth")
        reth.start()

        # wait for reth to start
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > 0,
            error_with="reth did not start in time",
            timeout=5,
        )

        # ensure reth db was reset to shorter chain
        assert int(rethrpc.eth_blockNumber(), base=16) == orig_blocknumber

        print("start sequencer")
        seq.start()

        print("wait for sync")
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > final_blocknumber,
            error_with="not syncing blocks",
            timeout=10,
        )
