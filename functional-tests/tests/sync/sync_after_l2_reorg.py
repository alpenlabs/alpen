import flexitest

from envs import net_settings, testenv
from utils import *


@flexitest.register
class FullnodeSyncAfterReorgTest(testenv.StrataTester):
    """This tests sync when el is missing blocks"""

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.HubNetworkEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("seq_node")
        seq_signer = ctx.get_service("sequencer_signer")
        fullnode = ctx.get_service("follower_1_node")
        reth = ctx.get_service("seq_reth")
        fnreth = ctx.get_service("follower_1_reth")

        seqrpc = seq.create_rpc()
        rethrpc = reth.create_rpc()
        fnrpc = fullnode.create_rpc()
        fnrethrpc = fnreth.create_rpc()

        wait_for_genesis(seqrpc, timeout=20)

        wait_until_epoch_finalized(seqrpc, 0, timeout=30)
        wait_until_epoch_finalized(fnrpc, 0, timeout=30)

        # ensure there are some blocks generated
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > 0,
            error_with="not building blocks",
            timeout=5,
        )

        print("stop sequencer block production")
        seq_signer.stop()

        # wait for fullnode to sync up
        wait_until(
            lambda: fnrpc.strata_syncStatus()["tip_height"]
            == seqrpc.strata_syncStatus()["tip_height"],
            error_with="fullnode did not sync with sequencer",
            timeout=5,
        )

        orig_blocknumber = seqrpc.strata_syncStatus()["tip_height"]
        print(f"stop seq @{orig_blocknumber}")
        fullnode.stop()
        seq.stop()

        # take snapshot of sequencer db as fork point
        SNAPSHOT_IDX = 1
        seq.snapshot_datadir(SNAPSHOT_IDX)

        print("restart sequencer")
        seq.start()
        seq_signer.start()

        # generate more blocks
        wait_until(
            lambda: int(rethrpc.eth_blockNumber(), base=16) > orig_blocknumber + 2,
            error_with="not building blocks",
            timeout=5,
        )

        fullnode.start()

        print("stop sequencer block production")
        seq_signer.stop()

        # wait for fullnode to sync up
        wait_until(
            lambda: fnrpc.strata_syncStatus()["tip_height"]
            == seqrpc.strata_syncStatus()["tip_height"],
            error_with="fullnode did not sync with sequencer",
            timeout=5,
        )

        final_blocknumber = seqrpc.strata_syncStatus()["tip_height"]
        print(f"stop seq @{final_blocknumber}")

        orig_el_blockhash = rethrpc.eth_getBlockByNumber(hex(final_blocknumber), False)["hash"]
        orig_el_blockhash_fn = fnrethrpc.eth_getBlockByNumber(hex(final_blocknumber), False)["hash"]

        assert orig_el_blockhash == orig_el_blockhash_fn, "seq and fn EE should be in sync"

        fullnode.stop()
        seq.stop()

        # replace sequencer db with older snapshot with shorter chain at fork point to trigger reorg
        seq.restore_snapshot(SNAPSHOT_IDX)

        print("restart sequencer after chain revert")
        seq.start()
        fullnode.start()

        wait_until(
            lambda: seqrpc.strata_syncStatus()["tip_height"] > 0,
            error_with="reth did not start in time",
            timeout=5,
        )
        # ensure sequencer db was reset to shorter chain
        assert seqrpc.strata_syncStatus()["tip_height"] < final_blocknumber, (
            "sequencer should have shorter chain"
        )
        # ensure fullnode still has original chain
        assert fnrpc.strata_syncStatus()["tip_height"] == final_blocknumber, (
            "fullnode should be on same chain"
        )

        # resume block production for reorg'd chain
        seq_signer.start()

        print("wait for block production to resume")
        wait_until(
            lambda: seqrpc.strata_syncStatus()["tip_height"] > final_blocknumber,
            error_with="not syncing blocks",
            timeout=10,
        )

        new_el_blockhash = rethrpc.eth_getBlockByNumber(hex(final_blocknumber), False)["hash"]
        print(final_blocknumber, orig_el_blockhash, new_el_blockhash)

        assert orig_el_blockhash != new_el_blockhash, "sequencer EE should move to new fork"

        # wait for fullnode to sync up
        wait_until(
            lambda: fnrpc.strata_syncStatus()["tip_height"] > final_blocknumber,
            error_with="fullnode not syncing reorged blocks",
            timeout=10,
        )

        new_el_blockhash_fn = fnrethrpc.eth_getBlockByNumber(hex(final_blocknumber), False)["hash"]

        assert new_el_blockhash == new_el_blockhash_fn, (
            "seq and fn EE should have same block from reorg'd chain"
        )
