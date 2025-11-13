//! Verified auxiliary data.
//!
//! Contains verified auxiliary data for subprotocols during the processing phase.

use std::collections::HashMap;

use bitcoin::{Transaction, Txid};
use strata_btc_types::RawBitcoinTx;
use strata_merkle::CompactMmr64;

use crate::{
    AsmHasher, AuxError, AuxResult, Hash32,
    aux::data::{AuxData, ManifestLeafWithProof},
};

/// Contains verified auxiliary data for subprotocols during transaction processing.
///
/// This struct verifies all auxiliary data upfront during construction and stores
/// it in efficient lookup structures for O(1) access:
///
/// - **Bitcoin transactions**: Decoded and indexed by txid in a hashmap
/// - **Manifest leaves**: MMR proofs verified and indexed by MMR position
///
/// All verification happens during construction via [`try_new`](Self::try_new), so
/// subsequent getter method calls return already-verified data without additional
/// validation overhead.
#[derive(Debug, Clone)]
pub struct VerifiedAuxData {
    /// Verified Bitcoin transactions indexed by txid
    txs: HashMap<Txid, Transaction>,
    /// Verified manifest leaves indexed by MMR index
    manifest_leaves: HashMap<u64, Hash32>,
}

impl VerifiedAuxData {
    /// Attempts to create new verified auxiliary data by verifying and indexing all inputs.
    ///
    /// Decodes and verifies all Bitcoin transactions and manifest leaves from the provided
    /// unverified data. If any verification fails, returns an error and nothing is created.
    ///
    /// # Arguments
    ///
    /// * `data` - Unverified auxiliary data containing Bitcoin transactions and manifest leaves
    /// * `compact_mmr` - Compact MMR snapshot used to verify manifest leaf proofs
    ///
    /// # Errors
    ///
    /// Returns `AuxError::InvalidBitcoinTx` if any transaction fails to decode or is malformed.
    /// Returns `AuxError::InvalidMmrProof` if any manifest leaf's MMR proof fails verification.
    pub fn try_new(data: &AuxData, compact_mmr: &CompactMmr64<[u8; 32]>) -> AuxResult<Self> {
        let txs = Self::verify_and_index_bitcoin_txs(&data.bitcoin_txs)?;
        let manifest_leaves =
            Self::verify_and_index_manifest_leaves(&data.manifest_leaves, compact_mmr)?;

        Ok(Self {
            txs,
            manifest_leaves,
        })
    }

    /// Verifies and indexes Bitcoin transactions.
    ///
    /// Decodes raw Bitcoin transactions and indexes them by their txid.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::InvalidBitcoinTx` if any transaction fails to decode.
    fn verify_and_index_bitcoin_txs(
        raw_txs: &[RawBitcoinTx],
    ) -> AuxResult<HashMap<Txid, Transaction>> {
        let mut txs = HashMap::with_capacity(raw_txs.len());

        for (index, raw_tx) in raw_txs.iter().enumerate() {
            let tx: Transaction = raw_tx
                .try_into()
                .map_err(|source| AuxError::InvalidBitcoinTx { index, source })?;
            let txid = tx.compute_txid();
            txs.insert(txid, tx);
        }

        Ok(txs)
    }

    /// Verifies and indexes manifest leaves using MMR proofs.
    ///
    /// Verifies each manifest leaf's MMR proof against the provided compact MMR
    /// and indexes verified leaves by their MMR position.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::InvalidMmrProof` if any proof fails verification.
    fn verify_and_index_manifest_leaves(
        leaves: &[ManifestLeafWithProof],
        compact_mmr: &CompactMmr64<[u8; 32]>,
    ) -> AuxResult<HashMap<u64, Hash32>> {
        let mut manifest_leaves = HashMap::with_capacity(leaves.len());

        for item in leaves {
            if !compact_mmr.verify::<AsmHasher>(&item.proof, &item.leaf) {
                return Err(AuxError::InvalidMmrProof {
                    index: item.proof.index(),
                    hash: item.leaf,
                });
            }
            manifest_leaves.insert(item.proof.index(), item.leaf);
        }

        Ok(manifest_leaves)
    }

    /// Gets a verified Bitcoin transaction by txid.
    ///
    /// Returns the transaction if it exists in the verified data index.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::BitcoinTxNotFound` if the requested txid is not found.
    pub fn get_bitcoin_tx(&self, txid: Txid) -> AuxResult<&Transaction> {
        self.txs
            .get(&txid)
            .ok_or(AuxError::BitcoinTxNotFound { txid })
    }

    /// Gets a verified manifest leaf by MMR index.
    ///
    /// Returns the leaf hash if it exists at the given index.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::ManifestLeafNotFound` if the leaf is not found at the given index.
    pub fn get_manifest_leaf(&self, index: u64) -> AuxResult<&Hash32> {
        self.manifest_leaves
            .get(&index)
            .ok_or(AuxError::ManifestLeafNotFound { index })
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use strata_btc_types::RawBitcoinTx;
    use strata_identifiers::Buf32;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{AsmCompactMmr, AsmMmr, AuxError};

    #[test]
    fn test_verified_aux_data_empty() {
        let mmr = AsmMmr::new(16);
        let compact: AsmCompactMmr = mmr.into();

        let aux_data = AuxData {
            manifest_leaves: vec![],
            bitcoin_txs: vec![],
        };

        let verified = VerifiedAuxData::try_new(&aux_data, &compact).unwrap();

        // Should return error for non-existent txid
        let txid: Buf32 = [0u8; 32].into();
        let result = verified.get_bitcoin_tx(Txid::from(txid));
        assert!(result.is_err());

        // Should return error for non-existent manifest leaf
        let result = verified.get_manifest_leaf(100);
        assert!(result.is_err());
    }

    #[test]
    fn test_verified_aux_data_bitcoin_tx() {
        let raw_tx: RawBitcoinTx = ArbitraryGenerator::new().generate();
        let tx: Transaction = raw_tx.clone().try_into().unwrap();
        let txid = tx.compute_txid().as_raw_hash().to_byte_array();

        let mmr = AsmMmr::new(16);
        let compact: AsmCompactMmr = mmr.into();

        let aux_data = AuxData {
            manifest_leaves: vec![],
            bitcoin_txs: vec![raw_tx],
        };

        let verified = VerifiedAuxData::try_new(&aux_data, &compact).unwrap();

        // Should successfully return the bitcoin tx
        let txid_buf: Buf32 = txid.into();
        let result = verified.get_bitcoin_tx(Txid::from(txid_buf)).unwrap();
        assert_eq!(result.compute_txid().as_raw_hash().to_byte_array(), txid);
    }

    #[test]
    fn test_verified_aux_data_bitcoin_tx_not_found() {
        let mmr = AsmMmr::new(16);
        let compact: AsmCompactMmr = mmr.into();

        let aux_data = AuxData {
            manifest_leaves: vec![],
            bitcoin_txs: vec![],
        };

        let verified = VerifiedAuxData::try_new(&aux_data, &compact).unwrap();

        // Should return error for non-existent txid
        let txid: Buf32 = [0xFF; 32].into();
        let result = verified.get_bitcoin_tx(Txid::from(txid));
        assert!(matches!(result, Err(AuxError::BitcoinTxNotFound { .. })));
    }
}
