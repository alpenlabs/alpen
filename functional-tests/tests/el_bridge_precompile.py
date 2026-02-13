import flexitest
from web3 import Web3
from web3._utils.events import get_event_data

from envs import net_settings, testenv
from mixins.bridge_out_precompile_contract_mixin import BridgePrecompileMixin
from utils import *
from utils.constants import PRECOMPILE_BRIDGEOUT_ADDRESS
from utils.wait.reth import RethWaiter

withdrawal_intent_event_abi = {
    "anonymous": False,
    "inputs": [
        {"indexed": False, "internalType": "uint64", "name": "amount", "type": "uint64"},
        {"indexed": False, "internalType": "bytes", "name": "destination", "type": "bytes"},
        {"indexed": False, "internalType": "uint32", "name": "preferredOperator", "type": "uint32"},
    ],
    "name": "WithdrawalIntentEvent",
    "type": "event",
}
event_signature_text = "WithdrawalIntentEvent(uint64,bytes,uint32)"


@flexitest.register
class ElBridgePrecompileTest(BridgePrecompileMixin):
    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
                auto_generate_blocks=True,
            )
        )

    def main(self, ctx: flexitest.RunContext):
        web3: Web3 = self.reth.create_web3()

        source = web3.address
        dest = web3.to_checksum_address(PRECOMPILE_BRIDGEOUT_ADDRESS)

        priv_keys = get_priv_keys(ctx)
        self.deposit(ctx, self.deployed_contract_receipt.contractAddress, priv_keys)

        assert web3.is_connected(), "cannot connect to reth"

        original_block_no = web3.eth.block_number
        original_bridge_balance = web3.eth.get_balance(dest)
        original_source_balance = web3.eth.get_balance(source)

        assert original_bridge_balance == 0

        cfg = ctx.env.rollup_cfg()
        deposit_amount = cfg.deposit_amount
        to_transfer_sats = deposit_amount * 10_000_000_000
        to_transfer_wei = to_transfer_sats  # Same value in wei
        dest_pk = "04db4c79cc3ffca26f51e21241b9332d646b0772dd7e98de9c1de6b10990cab80b"
        # Prepend B=0 (no operator preference) before the BOSD descriptor
        calldata = "0x00" + dest_pk

        txid = web3.eth.send_transaction(
            {
                "to": dest,
                "value": hex(to_transfer_sats),
                "from": source,
                "gas": hex(200000),
                "data": calldata,
            }
        )

        receipt_waiter = RethWaiter(web3.eth, self.logger, 60, 0.5)
        receipt = receipt_waiter.wait_until_tx_included_in_block(txid.hex())

        assert receipt.status == 1, "precompile transaction failed"
        assert len(receipt.logs) == 1, "no logs or invalid logs"

        event_signature_hash = web3.keccak(text=event_signature_text).hex()
        log = receipt.logs[0]
        assert web3.to_checksum_address(log.address) == dest
        assert log.topics[0].hex() == event_signature_hash
        event_data = get_event_data(web3.codec, withdrawal_intent_event_abi, log)

        assert event_data.args.amount == deposit_amount
        assert event_data.args.destination.hex() == dest_pk

        final_block_no = web3.eth.block_number
        final_bridge_balance = web3.eth.get_balance(dest)
        final_source_balance = web3.eth.get_balance(source)

        assert original_block_no < final_block_no, "not building blocks"
        assert final_bridge_balance == 0, "bridge out funds not burned"
        total_gas_price = receipt.gasUsed * receipt.effectiveGasPrice
        assert (
            final_source_balance == original_source_balance - to_transfer_wei - total_gas_price
        ), "final balance incorrect"
