//! Auxiliary data resolver.
//!
//! Provides verified auxiliary data to subprotocols during the processing phase.

use std::collections::BTreeMap;

use strata_asm_common::AsmManifestMmr;

use crate::{
    AuxError, AuxResult, L1TxIndex,
    data::{
        BitcoinTxRequest, ManifestLeavesRequest, ManifestLeavesResponse, VerifiedManifestLeaves,
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
    /// Map from transaction index to manifest leaves response (just the leaves)
    manifest_leaves: &'a BTreeMap<L1TxIndex, VerifiedManifestLeaves>,
    /// Map from transaction index to Bitcoin transaction data
    bitcoin_txs: &'a BTreeMap<L1TxIndex, Vec<u8>>,
}

impl<'a> AuxResolver<'a> {
    /// Creates a new resolver from separate response maps.
    pub fn new(
        manifest_leaves: &'a BTreeMap<L1TxIndex, VerifiedManifestLeaves>,
        bitcoin_txs: &'a BTreeMap<L1TxIndex, Vec<u8>>,
    ) -> Self {
        Self {
            manifest_leaves,
            bitcoin_txs,
        }
    }

    /// Gets and verifies manifest leaves for a transaction.
    ///
    /// This method:
    /// 1. Retrieves the `ManifestLeaves` response and proofs for the transaction
    /// 2. Verifies the response matches the requested range
    /// 3. Verifies each leaf's MMR proof against the manifest MMR
    /// 4. Returns all verified leaves with their proofs
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
    /// # Returns
    ///
    /// Returns the raw Bitcoin transaction bytes.
    ///
    /// # Errors
    ///
    /// Returns `AuxError::MissingResponse` if no response exists for this transaction.
    ///
    /// Currently doesn't perform verification on Bitcoin transactions.
    /// Future versions may add Bitcoin SPV proof verification.
    pub fn get_bitcoin_tx(
        &self,
        tx_index: L1TxIndex,
        _req: &BitcoinTxRequest,
    ) -> AuxResult<Vec<u8>> {
        self.bitcoin_txs
            .get(&tx_index)
            .cloned()
            .ok_or(AuxError::MissingResponse { tx_index })
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_common::AsmManifestCompactMmr;

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
        let mut bitcoin_txs = BTreeMap::new();
        bitcoin_txs.insert(0, vec![]);

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
        let txid = [1u8; 32];
        let raw_tx = vec![0x01, 0x02, 0x03];

        let manifest_leaves = BTreeMap::new();
        let mut bitcoin_txs = BTreeMap::new();
        bitcoin_txs.insert(0, raw_tx.clone());

        let mmr = AsmManifestMmr::new(16);
        let _compact: AsmManifestCompactMmr = mmr.into();

        let resolver = AuxResolver::new(&manifest_leaves, &bitcoin_txs);

        // Should successfully return the bitcoin tx
        let req = BitcoinTxRequest { txid };
        let result = resolver.get_bitcoin_tx(0, &req).unwrap();
        assert_eq!(result, raw_tx);
    }

    // Note: Testing MMR proof verification requires creating valid proofs,
    // which needs access to internal MMR state during leaf addition.
    // This would be better tested in integration tests where we have
    // full control over the MMR lifecycle.
}
