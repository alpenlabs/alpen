use alloy_consensus::TxReceipt;
use alloy_sol_types::SolEvent;
use alpen_reth_primitives::{WithdrawalIntent, WithdrawalIntentEvent};
use reth_primitives::{Receipt, TransactionSigned};
use revm_primitives::{alloy_primitives::Bloom, U256};
use strata_primitives::{bitcoin_bosd::Descriptor, buf::Buf32};

use crate::constants::BRIDGEOUT_PRECOMPILE_ADDRESS;

pub(crate) const fn u256_from(val: u128) -> U256 {
    U256::from_limbs([(val & ((1 << 64) - 1)) as u64, (val >> 64) as u64, 0, 0])
}

/// Number of wei per rollup BTC (1e18).
pub(crate) const WEI_PER_BTC: u128 = 1_000_000_000_000_000_000u128;

/// Number of wei per satoshi (1e10).
pub(crate) const WEI_PER_SAT: U256 = u256_from(10_000_000_000u128);

/// Converts wei to satoshis.
/// Returns a tuple of (satoshis, remainder_in_wei).
pub(crate) fn wei_to_sats(wei: U256) -> (U256, U256) {
    wei.div_rem(WEI_PER_SAT)
}

/// Extracts withdrawal intents from bridge-out events in transaction receipts.
/// Returns an iterator of [`WithdrawalIntent`]s.
///
/// # Note
///
/// A [`Descriptor`], if invalid does not create a [`WithdrawalIntent`].
pub fn withdrawal_intents<'a>(
    transactions: &'a [TransactionSigned],
    receipts: &'a [Receipt],
) -> impl Iterator<Item = WithdrawalIntent> + 'a {
    assert_eq!(
        transactions.len(),
        receipts.len(),
        "transactions and receipts must have the same length"
    );

    transactions
        .iter()
        .zip(receipts.iter())
        .flat_map(|(tx, receipt)| {
            let txid = Buf32((*tx.hash()).into());
            receipt.logs.iter().filter_map(move |log| {
                if log.address != BRIDGEOUT_PRECOMPILE_ADDRESS {
                    return None;
                }

                let event = WithdrawalIntentEvent::decode_log(log).ok()?;
                let destination = Descriptor::from_bytes(&event.destination).ok()?;

                Some(WithdrawalIntent {
                    amt: event.amount,
                    destination,
                    withdrawal_txid: txid,
                })
            })
        })
}

/// Accumulates logs bloom from all receipts in the execution output.
///
/// This is a general EVM function that combines blooms from all transaction receipts
/// into a single block-level bloom filter for efficient log filtering.
pub fn accumulate_logs_bloom(receipts: &[Receipt]) -> Bloom {
    let mut logs_bloom = Bloom::default();
    receipts.iter().for_each(|r| {
        logs_bloom.accrue_bloom(&r.bloom());
    });
    logs_bloom
}
