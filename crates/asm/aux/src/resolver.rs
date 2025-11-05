//! Auxiliary data resolver.
//!
//! Provides verified auxiliary data to subprotocols during the processing phase.

use std::collections::BTreeMap;

use strata_asm_common::{AsmManifestCompactMmr, AsmManifestMmr};

use crate::{AuxError, AuxResponseEnvelope, AuxResult, L1TxIndex, ManifestLeaf};

/// Provides verified auxiliary data to subprotocols during transaction processing.
///
/// The resolver takes auxiliary responses provided by workers and verifies
/// their MMR proofs before handing them to subprotocols. This ensures that
/// all auxiliary data is cryptographically verified against the manifest MMR
/// stored in the chain view state.
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
///         let leaves = aux_resolver.get_manifest_leaves(idx)?;
///
///         // Use the verified data
///         for leaf in &leaves {
///             let hash = leaf.hash();
///             // ... process hash
///         }
///     }
/// }
/// ```
#[derive(Debug)]
pub struct AuxResolver<'a> {
    /// Map from transaction index to its single auxiliary response
    responses: &'a BTreeMap<L1TxIndex, AuxResponseEnvelope>,
    /// Full MMR for verifying proofs
    manifest_mmr: AsmManifestMmr,
}

impl<'a> AuxResolver<'a> {
    /// Creates a new resolver.
    ///
    /// # Arguments
    ///
    /// * `responses` - Map from transaction indices to their auxiliary response
    /// * `manifest_mmr_compact` - Compact MMR from the chain view state
    ///
    /// The compact MMR is expanded into a full MMR for verification purposes.
    pub fn new(
        responses: &'a BTreeMap<L1TxIndex, AuxResponseEnvelope>,
        manifest_mmr_compact: &AsmManifestCompactMmr,
    ) -> Self {
        Self {
            responses,
            manifest_mmr: AsmManifestMmr::from(manifest_mmr_compact.clone()),
        }
    }

    /// Gets the single response envelope for a transaction.
    ///
    /// Returns `None` if no auxiliary data was requested for this transaction.
    pub fn get_response(&self, tx_index: L1TxIndex) -> Option<&AuxResponseEnvelope> {
        self.responses.get(&tx_index)
    }

    /// Gets and verifies manifest leaves for a transaction.
    ///
    /// This method:
    /// 1. Retrieves all `ManifestLeaves` responses for the transaction
    /// 2. Verifies each leaf's MMR proof against the manifest MMR
    /// 3. Returns all verified leaves
    ///
    /// # Errors
    ///
    /// Returns `AuxError::InvalidMmrProof` if any leaf's proof fails verification.
    /// Returns `AuxError::TypeMismatch` if the response contains non-leaf data.
    ///
    /// # Returns
    ///
    /// Returns an empty vector if no auxiliary data was requested for this transaction.
    pub fn get_manifest_leaves(&self, tx_index: L1TxIndex) -> AuxResult<Vec<ManifestLeaf>> {
        let Some(envelope) = self.responses.get(&tx_index) else {
            return Ok(Vec::new());
        };

        match envelope {
            AuxResponseEnvelope::ManifestLeaves { leaves, .. } => {
                let mut verified = Vec::with_capacity(leaves.len());
                for leaf in leaves {
                    // Verify MMR proof: manifest_hash must be in the MMR
                    self.verify_manifest_leaf(leaf)?;
                    verified.push(leaf.clone());
                }
                Ok(verified)
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
    pub fn get_bitcoin_tx(&self, tx_index: L1TxIndex) -> AuxResult<Option<Vec<u8>>> {
        let Some(envelope) = self.responses.get(&tx_index) else {
            return Ok(None);
        };

        if let AuxResponseEnvelope::BitcoinTx { raw_tx, .. } = envelope {
            return Ok(Some(raw_tx.clone()));
        }

        Ok(None)
    }

    /// Verifies a manifest leaf's MMR proof.
    ///
    /// Checks that the `manifest_hash` is committed in the manifest MMR
    /// using the provided proof.
    fn verify_manifest_leaf(&self, leaf: &ManifestLeaf) -> AuxResult<()> {
        if !self
            .manifest_mmr
            .verify(&leaf.mmr_proof, &leaf.manifest_hash)
        {
            return Err(AuxError::InvalidMmrProof {
                height: leaf.height,
                hash: leaf.manifest_hash,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolver_empty_responses() {
        let responses = BTreeMap::new();
        let mmr = AsmManifestMmr::new(16);
        let compact: AsmManifestCompactMmr = mmr.into();

        let resolver = AuxResolver::new(&responses, &compact);

        // Should return empty vec for non-existent tx
        let leaves = resolver.get_manifest_leaves(0).unwrap();
        assert!(leaves.is_empty());

        let btc_tx = resolver.get_bitcoin_tx(0).unwrap();
        assert!(btc_tx.is_none());
    }

    #[test]
    fn test_resolver_type_mismatch() {
        let mut responses = BTreeMap::new();
        responses.insert(0, AuxResponseEnvelope::bitcoin_tx([0u8; 32], vec![]));

        let mmr = AsmManifestMmr::new(16);
        let compact: AsmManifestCompactMmr = mmr.into();

        let resolver = AuxResolver::new(&responses, &compact);

        // Requesting manifest leaves but got bitcoin tx
        let result = resolver.get_manifest_leaves(0);
        assert!(matches!(result, Err(AuxError::TypeMismatch { .. })));
    }

    #[test]
    fn test_resolver_bitcoin_tx() {
        let txid = [1u8; 32];
        let raw_tx = vec![0x01, 0x02, 0x03];

        let mut responses = BTreeMap::new();
        responses.insert(0, AuxResponseEnvelope::bitcoin_tx(txid, raw_tx.clone()));

        let mmr = AsmManifestMmr::new(16);
        let compact: AsmManifestCompactMmr = mmr.into();

        let resolver = AuxResolver::new(&responses, &compact);

        // Should successfully return the bitcoin tx
        let result = resolver.get_bitcoin_tx(0).unwrap();
        assert_eq!(result, Some(raw_tx));
    }

    // Note: Testing MMR proof verification requires creating valid proofs,
    // which needs access to internal MMR state during leaf addition.
    // This would be better tested in integration tests where we have
    // full control over the MMR lifecycle.
}
