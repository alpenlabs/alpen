//! Auxiliary data provider.
//!
//! Provides verified auxiliary data to subprotocols during the processing phase.

use bitcoin::{Transaction, hashes::Hash};

use crate::{
    AsmMmr, AuxError, AuxResult, BitcoinTxError, BitcoinTxRequest, L1TxIndex, ManifestLeavesError,
    ManifestLeavesRequest, ManifestLeavesResponse, aux::data::AuxData,
};

/// Provides verified auxiliary data to subprotocols during transaction processing.
///
/// The provider is initialized with auxiliary responses from workers and verifies
/// them based on information contained in each request before serving them to
/// subprotocols. Verification methods vary by request type (e.g., MMR proofs for
/// manifest leaves, txid validation for Bitcoin transactions).
#[derive(Debug)]
pub struct AuxDataProvider<'a> {
    data: &'a AuxData,
}

impl<'a> AuxDataProvider<'a> {
    /// Creates a new provider from separate response maps.
    pub fn new(data: &'a AuxData) -> Self {
        Self { data }
    }

    /// Gets and verifies manifest leaves for a transaction.
    ///
    /// This method:
    /// 1. Retrieves the manifest leaves and proofs for the transaction
    /// 2. Verifies each leaf's MMR proof against the manifest MMR
    /// 3. Returns the verified leaves
    ///
    /// # Errors
    ///
    /// Returns `AuxError::MissingResponse` if no response exists for this transaction.
    /// Returns `ManifestLeavesError::LengthMismatch` if the response length doesn't match the
    /// requested range. Returns `ManifestLeavesError::InvalidMmrProof` if any leaf's proof
    /// fails verification.
    pub fn get_manifest_leaves(
        &self,
        tx_index: L1TxIndex,
        req: &ManifestLeavesRequest,
    ) -> AuxResult<ManifestLeavesResponse> {
        let Some(response) = self.data.manifest_leaves.get(&tx_index) else {
            return Err(AuxError::MissingResponse { tx_index });
        };

        // Verify response matches requested length
        let expected_len = (req.end_height - req.start_height + 1) as usize;
        if response.leaves.len() != expected_len {
            return Err(ManifestLeavesError::LengthMismatch {
                tx_index,
                expected: expected_len,
                found: response.leaves.len(),
            }
            .into());
        }

        // Expand compact MMR from request for verification
        let mmr_full = AsmMmr::from(req.manifest_mmr.clone());

        for i in 0..expected_len {
            let height = req.start_height + i as u64;
            let hash = response.leaves[i];
            let proof = &response.proofs[i];
            if !mmr_full.verify(proof, &hash) {
                return Err(ManifestLeavesError::InvalidMmrProof { height, hash }.into());
            }
        }

        Ok(ManifestLeavesResponse {
            leaves: response.leaves.clone(),
        })
    }

    /// Gets Bitcoin transaction data for a transaction.
    ///
    /// This decodes the provided raw transaction bytes, recomputes the
    /// transaction's txid (txid), and ensures it matches the requested `txid`.
    ///
    /// # Returns
    ///
    /// The decoded `bitcoin::Transaction`.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::MissingResponse` if no response exists for this transaction.
    /// Returns `BitcoinTxError::InvalidTx` if the transaction cannot be decoded.
    /// Returns `BitcoinTxError::TxidMismatch` if the computed txid doesn't match the requested one.
    ///
    /// Note: This does not perform SPV verification for the transaction.
    pub fn get_bitcoin_tx(
        &self,
        tx_index: L1TxIndex,
        req: &BitcoinTxRequest,
    ) -> AuxResult<Transaction> {
        let raw_tx = self
            .data
            .bitcoin_txs
            .get(&tx_index)
            .ok_or(AuxError::MissingResponse { tx_index })?;

        let tx: Transaction = raw_tx
            .try_into()
            .map_err(|source| BitcoinTxError::InvalidTx { tx_index, source })?;

        let txid = tx.compute_txid();
        let found = txid.as_raw_hash().to_byte_array();
        if found != req.txid {
            return Err(BitcoinTxError::TxidMismatch {
                expected: req.txid,
                found,
            }
            .into());
        }
        Ok(tx)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use strata_btc_types::RawBitcoinTx;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{AsmCompactMmr, AsmMmr};

    #[test]
    fn test_provider_empty_responses() {
        let manifest_leaves = BTreeMap::new();
        let bitcoin_txs = BTreeMap::new();
        let aux_data = AuxData {
            manifest_leaves,
            bitcoin_txs,
        };
        let mmr = AsmMmr::new(16);
        let compact = mmr.into();

        let provider = AuxDataProvider::new(&aux_data);

        // Should return error for non-existent tx
        let req = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: compact,
        };
        let result = provider.get_manifest_leaves(0, &req);
        assert!(matches!(result, Err(AuxError::MissingResponse { .. })));

        let btc_req = BitcoinTxRequest { txid: [0u8; 32] };
        let result = provider.get_bitcoin_tx(0, &btc_req);
        assert!(matches!(result, Err(AuxError::MissingResponse { .. })));
    }

    #[test]
    fn test_provider_missing_response() {
        let manifest_leaves = BTreeMap::new();
        let raw_tx: RawBitcoinTx = ArbitraryGenerator::new().generate();
        let mut bitcoin_txs = BTreeMap::new();
        bitcoin_txs.insert(0, raw_tx);

        let mmr = AsmMmr::new(16);
        let compact = mmr.into();

        let data = AuxData {
            manifest_leaves,
            bitcoin_txs,
        };
        let provider = AuxDataProvider::new(&data);

        // Requesting manifest leaves but only bitcoin tx exists
        let req = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: compact,
        };
        let result = provider.get_manifest_leaves(0, &req);
        assert!(matches!(result, Err(AuxError::MissingResponse { .. })));
    }

    #[test]
    fn test_provider_bitcoin_tx() {
        let raw_tx: RawBitcoinTx = ArbitraryGenerator::new().generate();
        let tx: Transaction = raw_tx.clone().try_into().unwrap();
        let txid = tx.compute_wtxid().as_raw_hash().to_byte_array();

        let manifest_leaves = BTreeMap::new();
        let mut bitcoin_txs = BTreeMap::new();
        bitcoin_txs.insert(0, raw_tx);

        let mmr = AsmMmr::new(16);
        let _compact: AsmCompactMmr = mmr.into();

        let data = AuxData {
            manifest_leaves,
            bitcoin_txs,
        };
        let provider = AuxDataProvider::new(&data);

        // Should successfully return the bitcoin tx
        let req = BitcoinTxRequest { txid };
        let result = provider.get_bitcoin_tx(0, &req).unwrap();
        assert_eq!(result, tx);
    }
}
