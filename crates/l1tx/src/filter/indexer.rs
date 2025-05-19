use bitcoin::{Block, Transaction};
use strata_primitives::{
    batch::SignedCheckpoint,
    indexed::Indexed,
    l1::{DepositInfo, DepositRequestInfo, DepositSpendInfo, WithdrawalFulfillmentInfo},
};

use super::{
    extract_da_blobs, extract_deposit_requests, find_deposit_spends,
    parse_valid_checkpoint_envelopes, try_parse_tx_as_withdrawal_fulfillment, try_parse_tx_deposit,
    TxFilterConfig,
};

/// Interface to handle storage of extracted information from a transaction.
pub trait TxVisitor {
    /// Output type collecting what we want to extract from a tx.
    type Output;

    /// Do stuffs with [`SignedCheckpoint`].
    fn visit_checkpoint(&mut self, _chkpt: SignedCheckpoint) {}

    /// Do stuffs with `DepositInfo`.
    fn visit_deposit(&mut self, _d: DepositInfo) {}

    /// Do stuffs with `DepositRequest`.
    fn visit_deposit_request(&mut self, _d: DepositRequestInfo) {}

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
    for ckpt in parse_valid_checkpoint_envelopes(tx, filter_config) {
        visitor.visit_checkpoint(ckpt);
    }

    for dp in try_parse_tx_deposit(tx, filter_config) {
        visitor.visit_deposit(dp);
    }

    for da in extract_da_blobs(tx, filter_config) {
        visitor.visit_da(da);
    }

    // TODO: maybe remove this later when we do not require deposit request ops?
    for dr in extract_deposit_requests(tx, filter_config) {
        visitor.visit_deposit_request(dr);
    }

    if let Some(info) = try_parse_tx_as_withdrawal_fulfillment(tx, filter_config) {
        visitor.visit_withdrawal_fulfillment(info);
    }

    for spend_info in find_deposit_spends(tx, filter_config) {
        visitor.visit_deposit_spend(spend_info);
    }

    visitor.finalize()
}

/// Generic no-op tx indexer that emits nothing for every tx but could
/// substitute for any type of visitor.
pub struct NopTxVisitorImpl<T>(::std::marker::PhantomData<T>);

impl<T> TxVisitor for NopTxVisitorImpl<T> {
    type Output = T;

    fn finalize(self) -> Option<Self::Output> {
        None
    }
}
