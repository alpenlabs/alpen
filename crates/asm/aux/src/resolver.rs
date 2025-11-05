//! Auxiliary data resolver.
//!
//! Provides verified auxiliary data to subprotocols during the processing phase.

use std::collections::BTreeMap;

use strata_asm_common::AsmManifestMmr;

use crate::{
    AuxError, AuxRequestSpec, AuxResponseEnvelope, AuxResult, BitcoinTxRequest, L1TxIndex,
    ManifestLeaves, ManifestLeavesRequest,
};

/// Provides verified auxiliary data to subprotocols during transaction processing.
///
/// The resolver takes auxiliary responses provided by workers and verifies
/// their MMR proofs before handing them to subprotocols. For manifest leaves,
/// the required compact manifest MMR is supplied via each `ManifestLeavesRequest`
/// and expanded locally for verification. This ensures that all auxiliary data
/// is cryptographically verified against the manifest MMR committed in state.
///
/// # Example
///
/// ```ignore
/// fn process_txs(
///     state: &mut Self::State,
///     txs: &[TxInputRef],
///     anchor_pre: &AnchorState,
///     aux_resolver: &AuxResolver,
///     relayer: &mut impl MsgRelayer,
///     params: &Self::Params,
/// ) {
///     for (idx, tx) in txs.iter().enumerate() {
///         // Get manifest leaves (automatically verified)
///         // Include the manifest MMR snapshot for verification
///         let mmr_compact = /* obtain from state */ todo!("compact MMR");
///         let req = ManifestLeavesRequest { start_height: 100, end_height: 200, manifest_mmr: mmr_compact };
///         let data = aux_resolver.get_manifest_leaves(idx, &req)?;
///
///         // Use the verified data
///         for hash in &data.leaves {
///             // ... process hash
///         }
///     }
/// }
/// ```
#[derive(Debug)]
pub struct AuxResolver<'a> {
    /// Map from transaction index to its single auxiliary response
    responses: &'a BTreeMap<L1TxIndex, AuxResponseEnvelope>,
}

impl<'a> AuxResolver<'a> {
    /// Creates a new resolver.
    ///
    /// # Arguments
    ///
    /// * `responses` - Map from transaction indices to their auxiliary response
    ///
    /// Note: MMR context for verification is provided per-request via
    /// `ManifestLeavesRequest.manifest_mmr`.
    pub fn new(responses: &'a BTreeMap<L1TxIndex, AuxResponseEnvelope>) -> Self {
        Self { responses }
    }

    /// Gets the single response envelope for a transaction, validating via typed getters.
    ///
    /// Returns `Ok(None)` if no response was provided for this transaction.
    pub fn get_response(
        &self,
        tx_index: L1TxIndex,
        spec: &AuxRequestSpec,
    ) -> AuxResult<Option<&AuxResponseEnvelope>> {
        let Some(envelope) = self.responses.get(&tx_index) else {
            return Ok(None);
        };

        match spec {
            AuxRequestSpec::ManifestLeaves(req) => {
                let _ = self.get_manifest_leaves(tx_index, req)?;
            }
            AuxRequestSpec::BitcoinTx(req) => {
                let _ = self.get_bitcoin_tx(tx_index, req)?;
            }
        }
        Ok(Some(envelope))
    }

    /// Gets and verifies manifest leaves for a transaction.
    ///
    /// This method:
    /// 1. Retrieves the `ManifestLeaves` response for the transaction
    /// 2. Verifies the response matches the requested range
    /// 3. Verifies each leaf's MMR proof against the manifest MMR
    /// 4. Returns all verified leaves
    ///
    /// # Errors
    ///
    /// Returns `AuxError::InvalidMmrProof` if any leaf's proof fails verification.
    /// Returns `AuxError::TypeMismatch` if the response contains non-leaf data.
    ///
    /// # Returns
    ///
    /// Returns empty leaves/proofs if no auxiliary data was requested for this transaction.
    pub fn get_manifest_leaves(
        &self,
        tx_index: L1TxIndex,
        req: &ManifestLeavesRequest,
    ) -> AuxResult<ManifestLeaves> {
        let Some(envelope) = self.responses.get(&tx_index) else {
            return Ok(ManifestLeaves {
                leaves: Vec::new(),
                proofs: Vec::new(),
            });
        };

        // Ensure response is manifest leaves and matches requested length
        match envelope {
            AuxResponseEnvelope::ManifestLeaves(data) => {
                let expected_len = (req.end_height - req.start_height + 1) as usize;
                if data.leaves.len() != expected_len || data.proofs.len() != expected_len {
                    return Err(AuxError::SpecMismatch {
                        tx_index,
                        details: format!("leaf/proof count mismatch: expected {}", expected_len),
                    });
                }

                // Expand compact MMR from request for verification
                let mmr_full = AsmManifestMmr::from(req.manifest_mmr.clone());

                for i in 0..expected_len {
                    let height = req.start_height + i as u64;
                    let hash = data.leaves[i];
                    let proof = &data.proofs[i];
                    if !mmr_full.verify(proof, &hash) {
                        return Err(AuxError::InvalidMmrProof { height, hash });
                    }
                }
                Ok(ManifestLeaves {
                    leaves: data.leaves.clone(),
                    proofs: data.proofs.clone(),
                })
            }
            other => Err(AuxError::TypeMismatch {
                tx_index,
                expected: "ManifestLeaves",
                found: other.variant_name(),
            }),
        }
    }

    /// Gets Bitcoin transaction data for a transaction.
    ///
    /// # Returns
    ///
    /// Returns `Ok(Some(raw_tx))` if a Bitcoin transaction response exists.
    /// Returns `Ok(None)` if no Bitcoin transaction was requested.
    ///
    /// # Errors
    ///
    /// Currently doesn't perform verification on Bitcoin transactions.
    /// Future versions may add Bitcoin SPV proof verification.
    pub fn get_bitcoin_tx(
        &self,
        tx_index: L1TxIndex,
        req: &BitcoinTxRequest,
    ) -> AuxResult<Option<Vec<u8>>> {
        let Some(envelope) = self.responses.get(&tx_index) else {
            return Ok(None);
        };

        match envelope {
            AuxResponseEnvelope::BitcoinTx(raw_tx) => Ok(Some(raw_tx.clone())),
            other => Err(AuxError::TypeMismatch {
                tx_index,
                expected: "BitcoinTx",
                found: other.variant_name(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_common::AsmManifestCompactMmr;

    use super::*;

    #[test]
    fn test_resolver_empty_responses() {
        let responses = BTreeMap::new();
        let mmr = AsmManifestMmr::new(16);
        let compact = mmr.into();

        let resolver = AuxResolver::new(&responses);

        // Should return empty data for non-existent tx
        let req = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: compact,
        };
        let data = resolver.get_manifest_leaves(0, &req).unwrap();
        assert!(data.leaves.is_empty());

        let btc_req = BitcoinTxRequest { txid: [0u8; 32] };
        let btc_tx = resolver.get_bitcoin_tx(0, &btc_req).unwrap();
        assert!(btc_tx.is_none());
    }

    #[test]
    fn test_resolver_type_mismatch() {
        let mut responses = BTreeMap::new();
        responses.insert(0, AuxResponseEnvelope::BitcoinTx(vec![]));

        let mmr = AsmManifestMmr::new(16);
        let compact = mmr.into();

        let resolver = AuxResolver::new(&responses);

        // Requesting manifest leaves but got bitcoin tx
        let req = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: compact,
        };
        let result = resolver.get_manifest_leaves(0, &req);
        assert!(matches!(result, Err(AuxError::TypeMismatch { .. })));
    }

    #[test]
    fn test_resolver_bitcoin_tx() {
        let txid = [1u8; 32];
        let raw_tx = vec![0x01, 0x02, 0x03];

        let mut responses = BTreeMap::new();
        responses.insert(0, AuxResponseEnvelope::BitcoinTx(raw_tx.clone()));

        let mmr = AsmManifestMmr::new(16);
        let _compact: AsmManifestCompactMmr = mmr.into();

        let resolver = AuxResolver::new(&responses);

        // Should successfully return the bitcoin tx
        let req = BitcoinTxRequest { txid };
        let result = resolver.get_bitcoin_tx(0, &req).unwrap();
        assert_eq!(result, Some(raw_tx));
    }

    // Note: Testing MMR proof verification requires creating valid proofs,
    // which needs access to internal MMR state during leaf addition.
    // This would be better tested in integration tests where we have
    // full control over the MMR lifecycle.
}
