use bitcoin::Transaction;
use strata_asm_types::DepositSpendInfo;
use strata_primitives::l1::BitcoinOutPoint;

pub mod checkpoint;
pub mod indexer;
pub mod types;

use checkpoint::parse_valid_checkpoint_envelope;

use crate::filter::types::TxFilterConfig;

/// Parse da blobs from [`Transaction`].
fn extract_da_blobs<'a>(
    _tx: &'a Transaction,
    _filter_conf: &TxFilterConfig,
) -> impl Iterator<Item = impl Iterator<Item = &'a [u8]> + 'a> {
    // TODO: actually implement this when we have da
    std::iter::empty::<std::slice::Iter<'a, &'a [u8]>>().map(|inner| inner.copied())
}

/// Parse transaction and filter out any deposits that have been spent.
fn find_deposit_spends<'tx>(
    tx: &'tx Transaction,
    filter_conf: &'tx TxFilterConfig,
) -> impl Iterator<Item = DepositSpendInfo> + 'tx {
    tx.input.iter().filter_map(|txin| {
        let prevout = BitcoinOutPoint::new(txin.previous_output.txid, txin.previous_output.vout);
        filter_conf
            .expected_outpoints
            .get(&prevout)
            .map(|config| DepositSpendInfo {
                deposit_idx: config.deposit_idx,
            })
    })
}
