use bitcoin::{Block, Transaction};
use strata_asm_stf::group_txs_by_subprotocol;
use strata_asm_txs_bridge_v1::{
    deposit::{parse_deposit_tx, validate_deposit_output_lock, validate_drt_spending_signature},
    withdrawal_fulfillment::parse_withdrawal_fulfillment_tx,
    BRIDGE_V1_SUBPROTOCOL_ID,
};
use strata_asm_types::{DepositInfo, DepositSpendInfo, WithdrawalFulfillmentInfo};
use strata_checkpoint_types::SignedCheckpoint;
use strata_primitives::indexed::Indexed;

use super::{
    extract_da_blobs, find_deposit_spends, parse_valid_checkpoint_envelope, TxFilterConfig,
};
use crate::utils::{
    convert_bridge_v1_deposit_to_protocol_deposit,
    convert_bridge_v1_withdrawal_to_protocol_withdrawal,
};

/// Interface to handle storage of extracted information from a transaction.
pub trait TxVisitor {
    /// Output type collecting what we want to extract from a tx.
    type Output;

    /// Do stuffs with [`SignedCheckpoint`].
    fn visit_checkpoint(&mut self, _chkpt: SignedCheckpoint) {}

    /// Do stuffs with `DepositInfo`.
    fn visit_deposit(&mut self, _d: DepositInfo) {}

    /// Do stuffs with DA.
    fn visit_da<'a>(&mut self, _d: impl Iterator<Item = &'a [u8]>) {}

    /// Do stuffs with withdrawal fulfulment transactions
    fn visit_withdrawal_fulfillment(&mut self, _info: WithdrawalFulfillmentInfo) {}

    /// Do stuff with spent deposits
    fn visit_deposit_spend(&mut self, _info: DepositSpendInfo) {}

    /// Export the indexed data, if it rose to the level of being useful.
    fn finalize(self) -> Option<Self::Output>;
}

/// Extracts a list of interesting transactions from a block according to a
/// provided visitor, with parts extracted from a provided filter config.
pub fn index_block<V: TxVisitor>(
    block: &Block,
    visitor_fn: impl Fn() -> V,
    config: &TxFilterConfig,
) -> Vec<Indexed<V::Output>> {
    block
        .txdata
        .iter()
        .enumerate()
        .filter_map(|(i, tx)| {
            index_tx(tx, visitor_fn(), config).map(|outp| Indexed::new(i as u32, outp))
        })
        .collect::<Vec<_>>()
}

fn index_tx<V: TxVisitor>(
    tx: &Transaction,
    mut visitor: V,
    filter_config: &TxFilterConfig,
) -> Option<V::Output> {
    let tag = filter_config.deposit_config.magic_bytes;

    if let Some(tx_refs) = group_txs_by_subprotocol(tag, [tx]).remove(&BRIDGE_V1_SUBPROTOCOL_ID) {
        for tx_input in tx_refs {
            if let Ok(dp) =
                parse_deposit_tx(&tx_input).map(convert_bridge_v1_deposit_to_protocol_deposit)
            {
                if validate_deposit_output_lock(tx_input.tx(),  &filter_config.deposit_config.operators_pubkey).is_ok() {
                    visitor.visit_deposit(dp);
                }
            }

            if let Ok(bridge_v1_withdrawal) = parse_withdrawal_fulfillment_tx(&tx_input) {
                let wf = convert_bridge_v1_withdrawal_to_protocol_withdrawal(
                    bridge_v1_withdrawal,
                    tx.compute_txid(),
                );
                visitor.visit_withdrawal_fulfillment(wf);
            }
        }
    }

    if let Some(ckpt) = parse_valid_checkpoint_envelope(tx, filter_config) {
        visitor.visit_checkpoint(ckpt);
    }

    for da in extract_da_blobs(tx, filter_config) {
        visitor.visit_da(da);
    }

    for spend_info in find_deposit_spends(tx, filter_config) {
        visitor.visit_deposit_spend(spend_info);
    }

    visitor.finalize()
}

/// Generic no-op tx indexer that emits nothing for every tx but could
/// substitute for any type of visitor.
#[derive(Debug)]
pub struct NopTxVisitorImpl<T>(::std::marker::PhantomData<T>);

impl<T> TxVisitor for NopTxVisitorImpl<T> {
    type Output = T;

    fn finalize(self) -> Option<Self::Output> {
        None
    }
}
