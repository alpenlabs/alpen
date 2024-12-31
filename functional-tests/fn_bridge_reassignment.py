import time

import flexitest
from strata_utils import get_balance

import testenv
from constants import UNSPENDABLE_ADDRESS
from rollup_params_cfg import RollupConfig
from utils import get_bridge_pubkey, wait_until, wait_until_with_value


@flexitest.register
class BridgeWithdrawReassignmentTest(testenv.BridgeTestBase):
    """
    Makes two DRT deposits, then triggers the withdrawal.
    The bridge client associated with assigned operator id is stopped.
    After the dispatch assignment duration is over,
    Check if new operator is being assigned or not
    Ensure that the withdrawal resumes and completes successfully
    """

    def __init__(self, ctx: flexitest.InitContext):
        ctx.set_env(
            testenv.BasicEnvConfig(
                101, n_operators=3, pre_fund_addrs=True, duty_timeout_duration=10
            )
        )

    def main(self, ctx: flexitest.RunContext):
        address = ctx.env.gen_ext_btc_address()
        withdraw_address = ctx.env.gen_ext_btc_address()
        el_address = self.eth_account.address
        self.debug(f"Address: {address}")
        self.debug(f"Change Address: {withdraw_address}")
        self.debug(f"EL Address: {el_address}")

        cfg: RollupConfig = ctx.env.rollup_cfg()
        # D BTC
        deposit_amount = cfg.deposit_amount
        # BTC Operator's fee for withdrawal
        operator_fee = cfg.operator_fee
        # BTC extra fee for withdrawal
        withdraw_extra_fee = cfg.withdraw_extra_fee
        # dispatch assignment duration for reassignment
        dispatch_assignment_duration = cfg.dispatch_assignment_dur

        btc_url = self.btcrpc.base_url
        btc_user = self.btc.get_prop("rpc_user")
        btc_password = self.btc.get_prop("rpc_password")
        bridge_pk = get_bridge_pubkey(self.seqrpc)
        self.debug(f"Bridge pubkey: {bridge_pk}")

        original_balance = get_balance(withdraw_address, btc_url, btc_user, btc_password)
        self.debug(f"BTC balance before withdraw: {original_balance}")

        # Check initial balance is 0
        balance = int(self.rethrpc.eth_getBalance(el_address), 16)
        assert balance == 0, "Strata balance is not expected (should be zero initially)"

        # Perform two deposits
        self.deposit(ctx, el_address, bridge_pk)
        self.deposit(ctx, el_address, bridge_pk)

        # withdraw
        self.withdraw(ctx, el_address, withdraw_address)

        new_balance = get_balance(withdraw_address, btc_url, btc_user, btc_password)
        self.debug(f"BTC balance after withdraw: {new_balance}")

        # Check assigned operator
        duties = self.seqrpc.strata_getBridgeDuties(0, 0)["duties"]
        withdraw_duty = [d for d in duties if d["type"] == "FulfillWithdrawal"][0]
        assigned_op_idx = withdraw_duty["payload"]["assigned_operator_idx"]
        assigned_operator = ctx.get_service(f"bridge.{assigned_op_idx}")
        self.debug(f"Assigned operator index: {assigned_op_idx}")

        # Stop assigned operator
        self.debug("Stopping assigned operator ...")
        assigned_operator.stop()

        # Let enough blocks pass so the assignment times out
        self.btcrpc.proxy.generatetoaddress(dispatch_assignment_duration, UNSPENDABLE_ADDRESS)
        time.sleep(3)

        # Re-check duties
        duties = self.seqrpc.strata_getBridgeDuties(0, 0)["duties"]
        withdraw_duty = [d for d in duties if d["type"] == "FulfillWithdrawal"][0]
        new_assigned_op_idx = withdraw_duty["payload"]["assigned_operator_idx"]
        new_assigned_operator = ctx.get_service(f"bridge.{new_assigned_op_idx}")
        self.debug(f"new assigned operator is {new_assigned_op_idx}")

        # Ensure a new operator is assigned
        assert new_assigned_operator != assigned_operator, "No new operator was assigned"
        assigned_operator.start()
        bridge_rpc = assigned_operator.create_rpc()

        wait_until(lambda: bridge_rpc.stratabridge_uptime() is not None, timeout=10)

        # generate l1 blocks equivalent to dispatch assignment duration
        self.btcrpc.proxy.generatetoaddress(dispatch_assignment_duration, UNSPENDABLE_ADDRESS)

        difference = deposit_amount - operator_fee - withdraw_extra_fee
        new_balance = wait_until_with_value(
            lambda: get_balance(withdraw_address, btc_url, btc_user, btc_password),
            predicate=lambda v: v == original_balance + difference,
            timeout=20,
        )

        self.debug(f"BTC balance after stopping and starting again: {new_balance}")
        return True
