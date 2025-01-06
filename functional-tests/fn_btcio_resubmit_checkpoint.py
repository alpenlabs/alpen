import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

import testenv
from utils import (
    generate_n_blocks,
    get_envelope_pushdata,
    submit_da_blob,
    wait_until,
    wait_until_with_value,
)


@flexitest.register
class ResubmitCheckpointTest(testenv.StrataTester):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        btc = ctx.get_service("bitcoin")
        seq = ctx.get_service("sequencer")
        btcrpc: BitcoindClient = btc.create_rpc()
        seqrpc = seq.create_rpc()

        # generate 5 btc blocks
        generate_n_blocks(btcrpc, 5)

        # Generate some funds to sequencer
        seqaddr = seq.get_prop("address")

        # Wait for seq
        wait_until(
            lambda: seqrpc.strata_protocolVersion() is not None,
            error_with="Sequencer did not start on time",
        )

        verified_on = wait_until_with_value(
            lambda: seqrpc.strata_getL2BlockStatus(1),
            predicate=lambda val: isinstance(val, dict) and "Finalized" in val,
            error_with="transactions are not being Finalized",
            timeout=10,
        )
        verified_block_hash = btcrpc.proxy.getblockhash(verified_on["Finalized"])
        block_data = btcrpc.getblock(verified_block_hash)
        envelope_data = ""
        for tx in block_data["txs"]:
            try:
                envelope_data = get_envelope_pushdata(tx.witness_data().hex())
            except ValueError:
                print("Not an envelope transaction")
                continue

        tx = submit_da_blob(btcrpc, seqrpc, envelope_data)
        # Calculate scriptbpubkey for sequencer address
        addrdata = btcrpc.proxy.validateaddress(seqaddr)
        scriptpubkey = addrdata["scriptPubKey"]

        # NOTE: could have just compared address
        # but bitcoinlib is somehow giving bc1* addr even though network is regtest
        assert (
            tx.outputs[0].lock_script.hex() == scriptpubkey
        ), "Output should be locked to sequencer's scriptpubkey"

        # ensure that client is still up and running
        wait_until(
            lambda: seqrpc.strata_protocolVersion() is not None,
            error_with="sequencer rpc is not working",
        )

        # check if chain tip is being increased
        cur_chain_tip = seqrpc.strata_clientStatus()["chain_tip_slot"]
        wait_until(
            lambda: seqrpc.strata_clientStatus()["chain_tip_slot"] > cur_chain_tip,
            "chain tip slot hasn't changed since resubmit of checkpoint blob",
        )

        return True
