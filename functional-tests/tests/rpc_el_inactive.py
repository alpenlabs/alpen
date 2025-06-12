import time

import flexitest
from web3 import Web3

from envs import testenv
from utils import wait_until


@flexitest.register
class SeqStatusElInactiveTest(testenv.StrataTestBase):
    """
    Test that checks the behavior of client RPC when reth is down and ability to produce blocks
    when reth is up again
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        seq = ctx.get_service("sequencer")
        reth = ctx.get_service("reth")
        # create sequencer RPC and wait until it is active
        seqrpc = seq.create_rpc()

        wait_until(
            lambda: seqrpc.strata_protocolVersion() is not None,
            error_with="Sequencer did not start on time",
        )

        # wait for reth to be connected
        web3: Web3 = reth.create_web3()
        wait_until(lambda: web3.is_connected(), error_with="Reth did not start properly")

        # send 3 transaction so that reth can start after being stopped
        to_transfer = 1_000_000
        dest = web3.to_checksum_address("0x0000000000000000000000000006000000000001")
        transfer_balance(web3, dest, to_transfer)
        transfer_balance(web3, dest, to_transfer)
        transfer_balance(web3, dest, to_transfer)

        wait_until(
            lambda: web3.eth.get_balance(dest) == to_transfer * 3,
            error_with="Balance transfer not successful",
            timeout=10,
        )
        reth.stop()

        assert not web3.is_connected(), "Reth did not stop"

        # check if rpc is still working
        wait_until(
            lambda: seqrpc.strata_clientStatus() is not None,
            error_with="RPC server of sequencer crashed",
        )

        cur_slot = seqrpc.strata_clientStatus()["chain_tip_slot"]
        # wait for 2 seconds to allow block production if any
        time.sleep(2)
        new_slot = seqrpc.strata_clientStatus()["chain_tip_slot"]

        # block production should halt
        assert cur_slot == new_slot, "Block production didn't halt"

        # check if new l1 blocks are being recognized
        cur_l1_height = seqrpc.strata_l1status()["cur_height"]
        wait_until(
            lambda: seqrpc.strata_l1status()["cur_height"] > cur_l1_height,
            error_with="L1 reader crashed after reth stopped",
        )

        # stop the sequencer
        seq.stop()
        time.sleep(2)  # Give time for sequencer to properly shut down.

        # start reth again
        reth.start()
        wait_until(lambda: web3.is_connected(), error_with="Reth did not start properly")

        # start sequencer again
        seq.start()
        wait_until(
            lambda: seqrpc.strata_protocolVersion() is not None,
            error_with="Sequencer did not start on time after reth restart",
        )

        # check if new blocks are being created again
        cur_slot = seqrpc.strata_clientStatus()["chain_tip_slot"]

        wait_until(
            lambda: seqrpc.strata_clientStatus()["chain_tip_slot"] > cur_slot,
            error_with="New blocks are not being created",
        )


def transfer_balance(web3: Web3, dest, to_transfer: int):
    source = web3.address
    web3.eth.send_transaction(
        {"to": dest, "value": hex(to_transfer), "gas": hex(100000), "from": source}
    )
