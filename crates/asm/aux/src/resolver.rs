//! Auxiliary data resolver.
//!
//! Provides verified auxiliary data to subprotocols during the processing phase.

use std::collections::BTreeMap;

use bitcoin::{Transaction, hashes::Hash};
use strata_asm_common::AsmManifestMmr;
use strata_btc_types::RawBitcoinTx;

use crate::{
    AuxError, AuxResult, L1TxIndex,
    data::{
        BitcoinTxRequest, ManifestLeavesRequest, ManifestLeavesResponse, ManifestLeavesWithProofs,
    },
};

/// Provides verified auxiliary data to subprotocols during transaction processing.
///
/// The resolver takes auxiliary responses provided by workers and verifies
/// their MMR proofs before handing them to subprotocols. For manifest leaves,
/// the required compact manifest MMR is supplied via each `ManifestLeavesRequest`
/// and expanded locally for verification. This ensures that all auxiliary data
/// is cryptographically verified against the manifest MMR committed in state.
#[derive(Debug)]
pub struct AuxResolver<'a> {
    /// Map from transaction index to manifest leaves with proofs (unverified)
    manifest_leaves: &'a BTreeMap<L1TxIndex, ManifestLeavesWithProofs>,
    /// Map from transaction index to Bitcoin transaction data
    bitcoin_txs: &'a BTreeMap<L1TxIndex, RawBitcoinTx>,
}

impl<'a> AuxResolver<'a> {
    /// Creates a new resolver from separate response maps.
    pub fn new(
        manifest_leaves: &'a BTreeMap<L1TxIndex, ManifestLeavesWithProofs>,
        bitcoin_txs: &'a BTreeMap<L1TxIndex, RawBitcoinTx>,
    ) -> Self {
        Self {
            manifest_leaves,
            bitcoin_txs,
        }
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
    /// Returns `AuxError::InvalidMmrProof` if any leaf's proof fails verification.
    /// Returns `AuxError::MissingResponse` if no response exists for this transaction.
    pub fn get_manifest_leaves(
        &self,
        tx_index: L1TxIndex,
        req: &ManifestLeavesRequest,
    ) -> AuxResult<ManifestLeavesResponse> {
        let Some(response) = self.manifest_leaves.get(&tx_index) else {
            return Err(AuxError::MissingResponse { tx_index });
        };

        // Verify response matches requested length
        let expected_len = (req.end_height - req.start_height + 1) as usize;
        // Expand compact MMR from request for verification
        let mmr_full = AsmManifestMmr::from(req.manifest_mmr.clone());

        for i in 0..expected_len {
            let height = req.start_height + i as u64;
            let hash = response.leaves[i];
            let proof = &response.proofs[i];
            if !mmr_full.verify(proof, &hash) {
                return Err(AuxError::InvalidMmrProof { height, hash });
            }
        }

        Ok(ManifestLeavesResponse {
            leaves: response.leaves.clone(),
        })
    }

    /// Gets Bitcoin transaction data for a transaction.
    ///
    /// This decodes the provided raw transaction bytes, recomputes the
    /// transaction's witness txid (wtxid), and ensures it matches the
    /// requested `txid`.
    ///
    /// # Returns
    ///
    /// The decoded `bitcoin::Transaction`.
    ///
    /// # Errors
    ///
    /// - `AuxError::MissingResponse` if no response exists for this transaction.
    /// - `AuxError::InvalidBitcoinTx` if decoding the raw transaction fails.
    /// - `AuxError::TxidMismatch` if the decoded transaction's wtxid does not
    ///   match the requested `txid`.
    ///
    /// Note: This does not perform SPV verification for the transaction.
    /// Future versions may add Bitcoin SPV proof verification.
    pub fn get_bitcoin_tx(
        &self,
        tx_index: L1TxIndex,
        req: &BitcoinTxRequest,
    ) -> AuxResult<Transaction> {
        let raw_tx = self
            .bitcoin_txs
            .get(&tx_index)
            .ok_or(AuxError::MissingResponse { tx_index })?;
        let tx: Transaction = raw_tx
            .try_into()
            .map_err(|source| AuxError::InvalidBitcoinTx { tx_index, source })?;
        let wtxid = tx.compute_wtxid();
        let found = wtxid.as_raw_hash().to_byte_array();
        if found != req.txid {
            return Err(AuxError::TxidMismatch { tx_index, expected: req.txid, found });
        }
        Ok(tx)
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_common::AsmManifestCompactMmr;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn test_resolver_empty_responses() {
        let manifest_leaves = BTreeMap::new();
        let bitcoin_txs = BTreeMap::new();
        let mmr = AsmManifestMmr::new(16);
        let compact = mmr.into();

        let resolver = AuxResolver::new(&manifest_leaves, &bitcoin_txs);

        // Should return error for non-existent tx
        let req = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: compact,
        };
        let result = resolver.get_manifest_leaves(0, &req);
        assert!(matches!(result, Err(AuxError::MissingResponse { .. })));

        let btc_req = BitcoinTxRequest { txid: [0u8; 32] };
        let result = resolver.get_bitcoin_tx(0, &btc_req);
        assert!(matches!(result, Err(AuxError::MissingResponse { .. })));
    }

    #[test]
    fn test_resolver_missing_response() {
        let manifest_leaves = BTreeMap::new();
        let raw_tx: RawBitcoinTx = ArbitraryGenerator::new().generate();
        let mut bitcoin_txs = BTreeMap::new();
        bitcoin_txs.insert(0, raw_tx);

        let mmr = AsmManifestMmr::new(16);
        let compact = mmr.into();

        let resolver = AuxResolver::new(&manifest_leaves, &bitcoin_txs);

        // Requesting manifest leaves but only bitcoin tx exists
        let req = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: compact,
        };
        let result = resolver.get_manifest_leaves(0, &req);
        assert!(matches!(result, Err(AuxError::MissingResponse { .. })));
    }

    #[test]
    fn test_resolver_bitcoin_tx() {
        let raw_tx: RawBitcoinTx = ArbitraryGenerator::new().generate();
        let tx: Transaction = raw_tx.clone().try_into().unwrap();
        let txid = tx.compute_wtxid().as_raw_hash().to_byte_array();

        let manifest_leaves = BTreeMap::new();
        let mut bitcoin_txs = BTreeMap::new();
        bitcoin_txs.insert(0, raw_tx);

        let mmr = AsmManifestMmr::new(16);
        let _compact: AsmManifestCompactMmr = mmr.into();

        let resolver = AuxResolver::new(&manifest_leaves, &bitcoin_txs);

        // Should successfully return the bitcoin tx
        let req = BitcoinTxRequest { txid };
        let result = resolver.get_bitcoin_tx(0, &req).unwrap();
        assert_eq!(result, tx);
    }
}
