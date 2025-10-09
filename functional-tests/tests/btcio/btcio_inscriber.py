import flexitest
from bitcoinlib.services.bitcoind import BitcoindClient

from envs import testenv
from utils import generate_n_blocks, submit_da_blob


@flexitest.register
class L1WriterTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("basic")

    def main(self, ctx: flexitest.RunContext):
        btc = ctx.get_service("bitcoin")
        seq = ctx.get_service("sequencer")
        btcrpc: BitcoindClient = btc.create_rpc()
        seqrpc = seq.create_rpc()

        # generate 5 btc blocks
        generate_n_blocks(btcrpc, 5)

        # Submit blob
        # blobdata is 32 bytes length
        blobdata = "2c4253d512da5bb4223f10e8e6017ede69cc63d6e6126916f4b68a1830b7f805"
        # blobdata now is 128 byes length
        # This is required by the sps-50 tagged data handling (minimal length is 124b).
        blobdata = blobdata * 4
        tx = submit_da_blob(btcrpc, seqrpc, blobdata)

        assert any([blobdata in w.hex() for w in tx.inputs[0].witnesses]), (
            "Tx should have submitted blobdata in its witness"
        )

        return True
