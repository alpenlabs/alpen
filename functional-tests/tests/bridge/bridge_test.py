import flexitest

from envs import net_settings, testenv
from factory.test_cli import (
    extract_p2tr_pubkey,
    get_address,
    xonlypk_to_descriptor,
)
from mixins import bridge_mixin
from utils import *
from utils.utils import retry_rpc_with_asm_backoff


@flexitest.register
class BridgeTest(bridge_mixin.BridgeMixin):
    """
    Bridge Test using Bridge Manager functionality
    """

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
        priv_keys = get_priv_keys(ctx)
        el_address = self.alpen_cli.l2_address()
        print("-----------------------")
        print(el_address)

        final_balance = int(self.rethrpc.eth_getBalance(el_address), 16)
        print(f"Initial EL balance: {final_balance}")

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

        # Create first deposit using improved bridge manager
        drt_tx_id_1, dt_tx_id_1 = self.deposit(ctx, el_address, priv_keys)
        print(f"First deposit: DRT={drt_tx_id_1}, DT={dt_tx_id_1}")

        # Create second deposit using improved bridge manager
        drt_tx_id_2, dt_tx_id_2 = self.deposit(ctx, el_address, priv_keys)
        print(f"Second deposit: DRT={drt_tx_id_2}, DT={dt_tx_id_2}")

        # Verify deposits are tracked
        deposits = retry_rpc_with_asm_backoff(
            lambda: self.seqrpc.strata_getCurrentDeposits(), timeout=30, step=1.0
        )
        print(f"Current deposits from RPC: {deposits}")

        # Create withdrawal using improved bridge manager (includes block generation and waiting)
        l2_tx_hash, _, total_gas_used = self.withdraw(el_address)
        print(f"Withdrawal L2 hash: {l2_tx_hash}, gas used: {total_gas_used}")

        # Use bridge manager to fulfill all withdrawals (includes synchronization)
        fulfillment_txids = self.fulfill_withdrawal_intents(ctx)
        print(f"Fulfillment txids: {fulfillment_txids}")

        # Check final state
        remaining_intents = retry_rpc_with_asm_backoff(
            lambda: self.seqrpc.strata_getCurWithdrawalAssignments(), timeout=30, step=1.0
        )
        print(f"Remaining withdrawal intents: {remaining_intents}")
        final_deposits = retry_rpc_with_asm_backoff(
            lambda: self.seqrpc.strata_getCurrentDeposits(), timeout=30, step=1.0
        )
        print(f"Final deposits: {final_deposits}")

        return True
