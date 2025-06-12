import flexitest
from web3 import Web3

from envs import testenv
from utils import wait_until


@flexitest.register
class FullnodeElBlockGenerationTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env("hub1")

    def main(self, ctx: flexitest.RunContext):
        seq_web3: Web3 = ctx.get_service("seq_reth").create_web3()
        fn_web3: Web3 = ctx.get_service("follower_1_reth").create_web3()

        # give some time for the sequencer to start up and generate blocks
        wait_until(lambda: seq_web3.eth.get_block_number() > 1)

        dest = fn_web3.to_checksum_address("deadf001900dca3ebeefdeadf001900dca3ebeef")

        # send tx to fullnode reth
        txid = fn_web3.eth.send_transaction(
            {
                "to": dest,
                "value": hex(1_000_000_000),
                "gas": hex(100000),
                "from": fn_web3.address,
            }
        )

        # expect receipt to be available from sequencer reth
        seq_web3.eth.wait_for_transaction_receipt(txid, timeout=5)
