//! Verified auxiliary data.
//!
//! Contains verified auxiliary data for subprotocols during the processing phase.

use std::collections::HashMap;

use bitcoin::{OutPoint, Transaction, TxOut, Txid};
use strata_btc_types::RawBitcoinTx;
use strata_merkle::CompactMmr64;

use crate::{
    AsmHasher, AuxError, AuxResult, Hash32,
    aux::data::{AuxData, VerifiableManifestHash},
};

/// Contains verified auxiliary data for subprotocols during transaction processing.
///
/// This struct verifies all auxiliary data upfront during construction and stores
/// it in efficient lookup structures for O(1) access:
///
/// - **Bitcoin transactions**: Decoded and indexed by txid in a hashmap
/// - **Manifest hashes**: MMR proofs verified and indexed by L1 block height
///
/// All verification happens during construction via [`try_new`](Self::try_new), so
/// subsequent getter method calls return already-verified data without additional
/// validation overhead.
#[derive(Debug, Clone)]
pub struct VerifiedAuxData {
    /// Verified Bitcoin transactions indexed by txid
    txs: HashMap<Txid, Transaction>,
    /// Verified manifest hashes indexed by L1 block height
    manifest_hashes: HashMap<u64, Hash32>,
}

impl VerifiedAuxData {
    /// Creates new verified auxiliary data from already-validated components.
    ///
    /// This constructor assumes the data has already been verified and simply
    /// wraps it in the `VerifiedAuxData` structure.
    ///
    /// # Arguments
    ///
    /// * `txs` - Pre-verified Bitcoin transactions indexed by txid
    /// * `manifest_hashes` - Pre-verified manifest hashes indexed by MMR position
    fn new(txs: HashMap<Txid, Transaction>, manifest_hashes: HashMap<u64, Hash32>) -> Self {
        Self {
            txs,
            manifest_hashes,
        }
    }

    /// Attempts to create new verified auxiliary data by verifying and indexing all inputs.
    ///
    /// Decodes and verifies all Bitcoin transactions and manifest hashes from the provided
    /// unverified data. If any verification fails, returns an error and nothing is created.
    ///
    /// # Arguments
    ///
    /// * `data` - Unverified auxiliary data containing Bitcoin transactions and manifest hashes
    /// * `compact_mmr` - Compact MMR snapshot used to verify manifest hash proofs
    ///
    /// # Errors
    ///
    /// Returns `AuxError::InvalidBitcoinTx` if any transaction fails to decode or is malformed.
    /// Returns `AuxError::InvalidMmrProof` if any manifest hash's MMR proof fails verification.
    pub fn try_new(data: &AuxData, compact_mmr: &CompactMmr64<[u8; 32]>) -> AuxResult<Self> {
        let txs = Self::verify_and_index_bitcoin_txs(data.bitcoin_txs())?;
        let manifest_hashes =
            Self::verify_and_index_manifest_hashes(data.manifest_hashes(), compact_mmr)?;

        Ok(Self::new(txs, manifest_hashes))
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

    /// Verifies and indexes manifest hashes using MMR proofs.
    ///
    /// Verifies each manifest hash's MMR proof against the provided compact MMR
    /// and indexes verified hashes by their L1 block height.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::InvalidMmrProof` if any proof fails verification.
    fn verify_and_index_manifest_hashes(
        hashes: &[VerifiableManifestHash],
        compact_mmr: &CompactMmr64<[u8; 32]>,
    ) -> AuxResult<HashMap<u64, Hash32>> {
        let mut manifest_hashes = HashMap::with_capacity(hashes.len());

        for item in hashes {
            if !compact_mmr.verify::<AsmHasher>(item.proof(), item.hash()) {
                return Err(AuxError::InvalidMmrProof {
                    index: item.proof().index(),
                    hash: *item.hash(),
                });
            }
            // Index by L1 block height, not MMR index, since consumers query by height
            manifest_hashes.insert(item.height(), *item.hash());
        }

        Ok(manifest_hashes)
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

    /// Returns the transaction output for the given outpoint.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::BitcoinTxNotFound` if the transaction is not found.
    pub fn get_bitcoin_txout(&self, outpoint: &OutPoint) -> AuxResult<&TxOut> {
        let tx = self.get_bitcoin_tx(outpoint.txid)?;
        tx.output
            .get(outpoint.vout as usize)
            .ok_or(AuxError::BitcoinTxOutNotFound {
                txid: outpoint.txid,
                vout: outpoint.vout,
            })
    }

    /// Gets a verified manifest hash by L1 block height.
    ///
    /// Returns the manifest hash if it exists for the given height.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::ManifestHashNotFound` if the hash is not found at the given height.
    pub fn get_manifest_hash(&self, height: u64) -> AuxResult<Hash32> {
        self.manifest_hashes
            .get(&height)
            .copied()
            .ok_or(AuxError::ManifestHashNotFound { height })
    }

    /// Gets a range of verified manifest hashes by their L1 block heights.
    ///
    /// Returns a vector of manifest hashes for the given height range (inclusive).
    ///
    /// # Errors
    ///
    /// Returns an error if any hash in the range is not found.
    pub fn get_manifest_hashes(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> AuxResult<Vec<Hash32>> {
        (start_height..=end_height)
            .map(|height| self.get_manifest_hash(height))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use strata_btc_types::RawBitcoinTx;
    use strata_identifiers::Buf32;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{AsmMmr, AuxError};

    #[test]
    fn test_verified_aux_data_empty() {
        let mmr = AsmMmr::new(16);
        let aux_data = AuxData::default();

        let verified = VerifiedAuxData::try_new(&aux_data, &mmr).unwrap();

        // Should return error for non-existent txid
        let txid: Buf32 = [0u8; 32].into();
        let result = verified.get_bitcoin_tx(Txid::from(txid));
        assert!(result.is_err());

        // Should return error for non-existent manifest hash
        let result = verified.get_manifest_hash(100);
        assert!(result.is_err());
    }

    #[test]
    fn test_verified_aux_data_bitcoin_tx() {
        let raw_tx: RawBitcoinTx = ArbitraryGenerator::new().generate();
        let tx: Transaction = raw_tx.clone().try_into().unwrap();
        let txid = tx.compute_txid().as_raw_hash().to_byte_array();

        let mmr = AsmMmr::new(16);
        let aux_data = AuxData::new(vec![], vec![raw_tx]);

        let verified = VerifiedAuxData::try_new(&aux_data, &mmr).unwrap();

        // Should successfully return the bitcoin tx
        let txid_buf: Buf32 = txid.into();
        let result = verified.get_bitcoin_tx(Txid::from(txid_buf)).unwrap();
        assert_eq!(result.compute_txid().as_raw_hash().to_byte_array(), txid);
    }

    #[test]
    fn test_verified_aux_data_bitcoin_tx_not_found() {
        let mmr = AsmMmr::new(16);
        let aux_data = AuxData::default();

        let verified = VerifiedAuxData::try_new(&aux_data, &mmr).unwrap();

        // Should return error for non-existent txid
        let txid: Buf32 = [0xFF; 32].into();
        let result = verified.get_bitcoin_tx(Txid::from(txid));
        assert!(matches!(result, Err(AuxError::BitcoinTxNotFound { .. })));
    }
}
