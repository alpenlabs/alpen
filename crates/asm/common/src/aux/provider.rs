//! Auxiliary data provider.
//!
//! Provides verified auxiliary data to subprotocols during the processing phase.

use std::collections::HashMap;

use bitcoin::Transaction;
use strata_identifiers::Buf32;
use strata_merkle::CompactMmr64;

use crate::{AsmHasher, AuxError, AuxResult, Hash32, aux::data::AuxData};

/// Provides auxiliary data to subprotocols during transaction processing.
///
/// The provider verifies all auxiliary data upfront during construction and stores
/// it in efficient lookup structures. Bitcoin transactions are validated and indexed
/// by txid, while manifest leaves have their MMR proofs verified and are indexed by
/// their MMR position.
#[derive(Debug, Clone)]
pub struct AuxDataProvider {
    /// Verified Bitcoin transactions indexed by txid
    txs: HashMap<Buf32, Transaction>,
    /// Verified manifest leaves indexed by MMR index
    manifest_leaves: HashMap<u64, Hash32>,
}

impl AuxDataProvider {
    /// Creates a new provider by verifying and indexing all auxiliary data.
    ///
    /// This method performs the following validation:
    /// 1. Decodes all Bitcoin transactions and indexes them by txid
    /// 2. Verifies all manifest leaf MMR proofs against the provided compact MMR
    /// 3. Indexes verified leaves by their MMR position
    ///
    /// # Errors
    ///
    /// Returns `AuxError::InvalidBitcoinTx` if any transaction fails to decode.
    /// Returns `AuxError::InvalidMmrProof` if any MMR proof fails verification.
    pub fn new(data: &AuxData, compact_mmr: &CompactMmr64<[u8; 32]>) -> AuxResult<Self> {
        let mut txs = HashMap::with_capacity(data.bitcoin_txs.len());
        let mut manifest_leaves = HashMap::with_capacity(data.manifest_leaves.len());

        // Decode and index all Bitcoin transactions
        for (index, tx) in data.bitcoin_txs.iter().enumerate() {
            let tx: Transaction = tx
                .try_into()
                .map_err(|source| AuxError::InvalidBitcoinTx { index, source })?;
            let txid = tx.compute_txid().into();
            txs.insert(txid, tx);
        }

        // Verify and index all manifest leaves
        for (index, (leaf, proof)) in data.manifest_leaves.iter().enumerate() {
            if !compact_mmr.verify::<AsmHasher>(proof, leaf) {
                return Err(AuxError::InvalidMmrProof { index, hash: *leaf });
            }
            manifest_leaves.insert(proof.index(), *leaf);
        }

        Ok(Self {
            txs,
            manifest_leaves,
        })
    }

    /// Gets a verified Bitcoin transaction by txid.
    ///
    /// Returns the transaction if it exists in the provider's index.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::BitcoinTxNotFound` if the requested txid is not found.
    pub fn get_bitcoin_tx(&self, txid: &[u8; 32]) -> AuxResult<&Transaction> {
        let txid_buf: Buf32 = (*txid).into();
        self.txs
            .get(&txid_buf)
            .ok_or(AuxError::BitcoinTxNotFound { txid: *txid })
    }

    /// Gets a verified manifest leaf by MMR index.
    ///
    /// Returns the leaf hash if it exists at the given index.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::ManifestLeafNotFound` if the leaf is not found at the given index.
    pub fn get_manifest_leaf(&self, index: u64) -> AuxResult<Hash32> {
        self.manifest_leaves
            .get(&index)
            .copied()
            .ok_or(AuxError::ManifestLeafNotFound { index })
    }

    /// Gets a range of verified manifest leaves by their MMR indices.
    ///
    /// Returns a vector of leaf hashes for the given index range (inclusive).
    ///
    /// # Errors
    ///
    /// Returns an error if any leaf in the range is not found.
    pub fn get_manifest_leaves(&self, start: u64, end: u64) -> AuxResult<Vec<Hash32>> {
        (start..=end)
            .map(|idx| self.get_manifest_leaf(idx))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use strata_btc_types::RawBitcoinTx;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{AsmCompactMmr, AsmMmr, AuxError};

    #[test]
    fn test_provider_empty_data() {
        let mmr = AsmMmr::new(16);
        let compact: AsmCompactMmr = mmr.into();

        let aux_data = AuxData {
            manifest_leaves: vec![],
            bitcoin_txs: vec![],
        };

        let provider = AuxDataProvider::new(&aux_data, &compact).unwrap();

        // Should return error for non-existent txid
        let result = provider.get_bitcoin_tx(&[0u8; 32]);
        assert!(result.is_err());

        // Should return error for non-existent manifest leaf
        let result = provider.get_manifest_leaf(100);
        assert!(result.is_err());
    }

    #[test]
    fn test_provider_bitcoin_tx() {
        let raw_tx: RawBitcoinTx = ArbitraryGenerator::new().generate();
        let tx: Transaction = raw_tx.clone().try_into().unwrap();
        let txid = tx.compute_txid().as_raw_hash().to_byte_array();

        let mmr = AsmMmr::new(16);
        let compact: AsmCompactMmr = mmr.into();

        let aux_data = AuxData {
            manifest_leaves: vec![],
            bitcoin_txs: vec![raw_tx],
        };

        let provider = AuxDataProvider::new(&aux_data, &compact).unwrap();

        // Should successfully return the bitcoin tx
        let result = provider.get_bitcoin_tx(&txid).unwrap();
        assert_eq!(result.compute_txid().as_raw_hash().to_byte_array(), txid);
    }

    #[test]
    fn test_provider_bitcoin_tx_not_found() {
        let mmr = AsmMmr::new(16);
        let compact: AsmCompactMmr = mmr.into();

        let aux_data = AuxData {
            manifest_leaves: vec![],
            bitcoin_txs: vec![],
        };

        let provider = AuxDataProvider::new(&aux_data, &compact).unwrap();

        // Should return error for non-existent txid
        let result = provider.get_bitcoin_tx(&[0xFF; 32]);
        assert!(matches!(result, Err(AuxError::BitcoinTxNotFound { .. })));
    }
}
