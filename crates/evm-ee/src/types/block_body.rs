//! EVM block body implementation.

use alloy_consensus::BlockBody;
use reth_primitives::TransactionSigned;
use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::ExecBlockBody;

use crate::codec_shims::{decode_rlp_with_length, encode_rlp_with_length};

/// Block body for EVM execution containing transactions.
///
/// Contains the transaction list for a block. Uses reth-compatible TransactionSigned
/// for proper type compatibility with the execution engine.
#[derive(Clone, Debug)]
pub struct EvmBlockBody {
    // Store the full BlockBody using reth's TransactionSigned type
    body: BlockBody<TransactionSigned>,
}

impl EvmBlockBody {
    /// Creates a new EvmBlockBody from a vector of transactions.
    pub fn new(transactions: Vec<TransactionSigned>) -> Self {
        Self {
            body: BlockBody {
                transactions,
                ommers: vec![],
                withdrawals: None,
            },
        }
    }

    /// Creates a new EvmBlockBody from an alloy BlockBody.
    pub fn from_alloy_body(body: BlockBody<TransactionSigned>) -> Self {
        Self { body }
    }

    /// Gets a reference to the transactions.
    pub fn transactions(&self) -> &[TransactionSigned] {
        &self.body.transactions
    }

    /// Gets a reference to the full body.
    pub fn body(&self) -> &BlockBody<TransactionSigned> {
        &self.body
    }

    /// Returns the number of transactions in the block.
    pub fn transaction_count(&self) -> usize {
        self.body.transactions.len()
    }
}

impl ExecBlockBody for EvmBlockBody {}

impl Codec for EvmBlockBody {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Encode transactions count
        let tx_count = self.body.transactions.len() as u32;
        tx_count.encode(enc)?;

        // Encode each transaction using RLP helper
        for tx in &self.body.transactions {
            encode_rlp_with_length(tx, enc)?;
        }

        // Encode withdrawals (optional)
        let has_withdrawals = self.body.withdrawals.is_some();
        has_withdrawals.encode(enc)?;

        if let Some(ref withdrawals) = self.body.withdrawals {
            let withdrawals_count = withdrawals.len() as u32;
            withdrawals_count.encode(enc)?;

            // Encode each withdrawal using RLP helper
            for withdrawal in withdrawals.iter() {
                encode_rlp_with_length(withdrawal, enc)?;
            }
        }

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode transactions count
        let tx_count = u32::decode(dec)? as usize;

        // Decode each transaction using RLP helper
        let mut transactions = Vec::with_capacity(tx_count);
        for _ in 0..tx_count {
            transactions.push(decode_rlp_with_length(dec)?);
        }

        // Decode withdrawals (optional)
        let has_withdrawals = bool::decode(dec)?;
        let withdrawals = if has_withdrawals {
            let withdrawals_count = u32::decode(dec)? as usize;
            let mut withdrawals_vec = Vec::with_capacity(withdrawals_count);

            // Decode each withdrawal using RLP helper
            for _ in 0..withdrawals_count {
                withdrawals_vec.push(decode_rlp_with_length(dec)?);
            }

            Some(withdrawals_vec)
        } else {
            None
        };

        Ok(Self {
            body: BlockBody {
                transactions,
                ommers: vec![], // Ommers are deprecated post-merge, always empty
                withdrawals: withdrawals.map(Into::into),
            },
        })
    }
}
