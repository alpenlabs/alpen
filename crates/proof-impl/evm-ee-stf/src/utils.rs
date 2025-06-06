use strata_primitives::{buf::Buf32, evm_exec::create_evm_extra_payload, l1::BitcoinAmount};
use strata_state::{
    block::ExecSegment,
    bridge_ops,
    exec_update::{ELDepositData, ExecUpdate, Op, UpdateInput, UpdateOutput},
};

use crate::EvmBlockStfOutput;

/// Generates an execution segment from the given EE-STF result.
pub fn generate_exec_update(el_proof_pp: &EvmBlockStfOutput) -> ExecSegment {
    let withdrawals = el_proof_pp
        .withdrawal_intents
        .iter()
        .map(|intent| {
            // TODO: proper error handling
            bridge_ops::WithdrawalIntent::new(
                BitcoinAmount::from_sat(intent.amt),
                intent.destination.clone(),
                intent.withdrawal_txid,
            )
        })
        .collect::<Vec<_>>();

    let applied_ops = el_proof_pp
        .deposit_requests
        .iter()
        .map(|request| {
            Op::Deposit(ELDepositData::new(
                request.index,
                gwei_to_sats(request.amount),
                request.address.as_slice().to_vec(),
            ))
        })
        .collect::<Vec<_>>();

    let update_input = UpdateInput::new(
        el_proof_pp.block_idx,
        applied_ops,
        Buf32(*el_proof_pp.txn_root),
        create_evm_extra_payload(Buf32(*el_proof_pp.new_blockhash)),
    );

    let update_output = UpdateOutput::new_from_state((*el_proof_pp.new_state_root).into())
        .with_withdrawals(withdrawals);

    let exec_update = ExecUpdate::new(update_input, update_output);

    ExecSegment::new(exec_update)
}

const fn gwei_to_sats(gwei: u64) -> u64 {
    // 1 BTC = 10^8 sats = 10^9 gwei
    gwei / 10
}
