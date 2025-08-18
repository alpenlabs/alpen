import os

import flexitest

from mixins import bridge_mixin
from utils import *
from envs import net_settings, testenv
from strata_utils import (
        create_deposit_transaction,
        extract_p2tr_pubkey,
        xonlypk_to_descriptor,
        get_address,
        create_withdrawal_fulfillment
)
import time


@flexitest.register
class BridgeSimpleDepositTest(bridge_mixin.BridgeMixin):
    """
    Simple Bridge Test
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                101,
                prover_client_settings=ProverClientSettings.new_with_proving(),
                rollup_settings=net_settings.get_fast_batch_settings(),
            )
        )

    def main(self, ctx: flexitest.RunContext):
        path = os.path.join(ctx.datadir_root, "_bridge_simple_test" ,"_init")
        print(path)
        priv_keys= []
        opkeys = sorted(
                filter(lambda file: file.startswith("opkey"), os.listdir(path)),
                key=lambda x: int(''.join(filter(str.isdigit, x))))
        print(opkeys)
        for filename in opkeys:
            if not filename.startswith("op"):
                continue

            full_path = os.path.join(path, filename)
            with open(full_path, "r") as f:
                content = f.read().strip()
                priv_keys.append(content)

        btc_url = self.btcrpc.base_url
        btc_user = self.btc.get_prop("rpc_user")
        btc_password = self.btc.get_prop("rpc_password")


        el_address = self.bridge_eth_account.address

        final_balance = int(self.rethrpc.eth_getBalance(el_address), 16)
        print(final_balance)

        self.debug(f"EL Address (without 0x): {el_address[2:]}")
        # Generate addresses
        address = ctx.env.gen_ext_btc_address()
        withdraw_address = ctx.env.gen_ext_btc_address()
        self.debug(f"Address: {address}")
        self.debug(f"Change Address: {withdraw_address}")
        self.debug(f"EL Address: {el_address}")

        withdraw_address = get_address(1)
        xonlypk = extract_p2tr_pubkey(withdraw_address)
        self.debug(f"XOnly PK: {xonlypk}")
        bosd = xonlypk_to_descriptor(xonlypk)
        self.debug(f"BOSD: {bosd}")


        bridge_pk = get_bridge_pubkey(self.seqrpc)
        print("---------------------------")
        print(f"Bridge PK: {bridge_pk}")
        print("---------------------------")


        # if not self.check_for_recent_bridge_in(el_address): Since now our bridge can handle multiple deposits we don't need this
        # self.debug("There was no recent bridge in, sending deposit request transaction")
        self.deposit(ctx, el_address, priv_keys)

        # Create the deposit request transaction
        tx = bytes(
            create_deposit_transaction(0, priv_keys)
        ).hex()
        # Send the transaction to the Bitcoin network
        dt_tx_id: str = self.btcrpc.proxy.sendrawtransaction(tx)
        print(f"Deposit Transaction ID {dt_tx_id}")

        seq_addr = self.seq.get_prop("address")
        self.btcrpc.proxy.generatetoaddress(6, seq_addr)
        val = self.seqrpc.strata_getCurrentDeposits()
        self.deposit(ctx, el_address, priv_keys)
        # Create the deposit request transaction
        tx = bytes(
            create_deposit_transaction(1, priv_keys)
        ).hex()
        dt_tx_id: str = self.btcrpc.proxy.sendrawtransaction(tx)
        print(f"Deposit Transaction ID {dt_tx_id}")

        self.btcrpc.proxy.generatetoaddress(6, seq_addr)
        time.sleep(10)
        val = self.seqrpc.strata_getCurrentDeposits()
        print(val)
        # now create a withdrawal
        self.withdraw(ctx, el_address, bosd)
        self.btcrpc.proxy.generatetoaddress(10, seq_addr)
        time.sleep(10)
        withdrawal_intents = self.seqrpc.strata_getWithdrawalIntent()
        print(withdrawal_intents)
        for intent in withdrawal_intents:
            tx = create_withdrawal_fulfillment(
                    intent['destination'],
                    intent['amt'],
                    intent['operator_idx'],
                    intent['deposit_idx'],
                    intent['deposit_txid'],
                    btc_url,
                    btc_user,
                    btc_password
                    )
            tx = bytes(tx).hex()
            wft_tx_id = self.btcrpc.proxy.sendrawtransaction(tx)
            print(wft_tx_id)

        self.btcrpc.proxy.generatetoaddress(10, seq_addr)
        time.sleep(4)
        withdrawal_intents = self.seqrpc.strata_getWithdrawalIntent()
        print(withdrawal_intents)

        # handle withdrawal now
        return True
