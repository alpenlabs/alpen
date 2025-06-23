import time

import flexitest
from web3 import Web3

from envs import testenv
from utils.reth import get_chainconfig
from utils.utils import RollupParamsSettings
from utils.wait import RethWaiter

BLOCK_GAS_LIMIT = 100_000
EPOCH_GAS_LIMIT = 200_000
GAS_PER_TX = 21_000
TX_COUNT = 10


chain_config = get_chainconfig()
chain_config["gasLimit"] = hex(BLOCK_GAS_LIMIT)


@flexitest.register
class ElBatchGasLimitTest(testenv.StrataTestBase):
    def __init__(self, ctx: flexitest.InitContext):
        # FIXME: running in strict mode to not cross epoch boundaries while testing
        rollup_settings = RollupParamsSettings.new_default().strict_mode()
        ctx.set_env(
            testenv.BasicEnvConfig(
                110,
                rollup_settings=rollup_settings,
                epoch_gas_limit=EPOCH_GAS_LIMIT,
                custom_chain=chain_config,
            )
        )

    def main(self, ctx: flexitest.RunContext):
        seq_signer = ctx.get_service("sequencer_signer")
        seq_signer.stop()
        # FIXME: process is NOT terminated immediately so need to wait
        time.sleep(1)

        reth = ctx.get_service("reth")
        rethrpc = reth.create_rpc()
        web3: Web3 = reth.create_web3()

        source = web3.address
        nonce = web3.eth.get_transaction_count(source)
        # send 10 txns with GAS_PER_TX gas each
        _txids = [make_burner_transaction(web3, nonce + i) for i in range(0, TX_COUNT)]
        # if all txns are included, epoch gas limit should be crossed
        assert GAS_PER_TX * TX_COUNT > EPOCH_GAS_LIMIT

        original_block_no = web3.eth.get_block_number()

        # re-start block production
        seq_signer.start()

        # we expect txns to be included in immediate next blocks
        # wait for txns to be included in new blocks until we get consecutive empty blocks
        # signifying either all txns are processed or epoch limit reached
        total_gas_used = 0
        block_no = original_block_no + 1
        zero_gas_blocks = 0
        reth_waiter = RethWaiter(rethrpc, timeout=30, interval=1)
        while zero_gas_blocks < 2:
            reth_waiter.wait_until_eth_block_at_least(block_no)

            header = web3.eth.get_block(block_no)
            self.info(f"block_number: {header['number']}, gas_used: {header['gasUsed']}")

            if header["gasUsed"] == 0:
                zero_gas_blocks += 1
            else:
                zero_gas_blocks = 0

            total_gas_used += header["gasUsed"]
            block_no += 1

        self.info(f"total gas used: {total_gas_used}")

        assert total_gas_used <= EPOCH_GAS_LIMIT, "epoch gas should be limited"
        assert total_gas_used < GAS_PER_TX * TX_COUNT, "all txns should NOT be processed"


def make_burner_transaction(web3: Web3, nonce: int):
    """
    :param web3: Web3 instance.
    :nonce: Nonce for the transaction.
    :return: Transaction id
    """

    tx_params = {
        "to": "0x0000000000000000000000000000000000000000",
        "value": Web3.to_wei(0.001, "ether"),
        "gas": GAS_PER_TX,
        "from": web3.address,
        "nonce": nonce,
    }
    txid = web3.eth.send_transaction(tx_params)
    return txid
