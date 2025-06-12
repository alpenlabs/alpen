import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from utils import (
    ProverClientSettings,
    RollupParamsSettings,
    generate_n_blocks,
    get_envelope_pushdata,
    submit_da_blob,
    wait_until,
    wait_until_with_value,
)


@flexitest.register
class ResubmitCheckpointTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        settings = RollupParamsSettings.new_default()
        settings.proof_timeout = 5
        ctx.set_env(
            testenv.BasicEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=settings,
            )
        )

    def main(self, ctx: flexitest.RunContext):
        btc = ctx.get_service("bitcoin")
        seq = ctx.get_service("sequencer")
        btcrpc: BitcoindClient = btc.create_rpc()
        seqrpc = seq.create_rpc()

        # generate 5 btc blocks
        generate_n_blocks(btcrpc, 5)

        # Wait for seq
        wait_until(
            lambda: seqrpc.strata_protocolVersion() is not None,
            error_with="Sequencer did not start on time",
        )

        verified_on = wait_until_with_value(
            lambda: seqrpc.strata_getL2BlockStatus(1),
            predicate=lambda val: isinstance(val, dict) and "Finalized" in val,
            error_with="transactions are not being Finalized",
            timeout=30,
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
