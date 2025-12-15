//! Auxiliary data resolver for ASM Worker.
//!
//! Resolves auxiliary data requests from subprotocols during pre-processing phase.
//! Fetches Bitcoin transactions and historical manifest hashes with MMR proofs.
//!
//! ### Manifest Hash Resolution
//!
//! Fully implemented with on-demand MMR proof generation:
//! - Maps L1 block heights to MMR indices using genesis offset
//! - Fetches manifest hashes from fast lookup storage
//! - Generates MMR proofs using `AsmDBSled`
//! - Converts `MerkleProofB32` (SSZ type) to `MerkleProof<Hash32>` (ASM type)
//!
//! ### Bitcoin Transaction Resolution
//!
//! Fully implemented using Bitcoin client's `getrawtransaction` RPC:
//! - Fetches raw transaction data by txid from Bitcoin node
//! - Requires Bitcoin node with transaction indexing enabled (`txindex=1`)
//! - Returns `WorkerError::BitcoinTxNotFound` if transaction not found

use std::sync::Arc;

use strata_asm_common::{
    AsmMerkleProof, AuxData, AuxRequests, Hash32, ManifestHashRange, VerifiableManifestHash,
};
use strata_btc_types::BitcoinTxid;
use strata_params::Params;
use strata_primitives::prelude::*;
use tracing::*;

use crate::{WorkerContext, WorkerError, WorkerResult};

/// Auxiliary data resolver that fetches external data required by subprotocols.
///
/// Resolves two types of auxiliary data:
/// 1. Bitcoin transactions by txid
/// 2. Historical manifest hashes with MMR proofs
///
/// Both resolution types are fully implemented:
/// - Bitcoin transaction fetching via Bitcoin RPC (requires txindex=1)
/// - MMR proof generation using AsmDBSled for on-demand proof generation
pub struct AuxDataResolver<'a> {
    /// Worker context for accessing ASM state and MMR database
    context: &'a dyn WorkerContext,
    /// Rollup parameters for genesis height calculation
    params: Arc<Params>,
}

impl<'a> AuxDataResolver<'a> {
    /// Creates a new auxiliary data resolver.
    ///
    /// # Arguments
    ///
    /// * `context` - Worker context for ASM state access and MMR database
    /// * `params` - Rollup parameters (needed for genesis height)
    pub fn new(context: &'a dyn WorkerContext, params: Arc<Params>) -> Self {
        Self { context, params }
    }

    /// Resolves all auxiliary data requests from subprotocols.
    ///
    /// This is the main entry point that coordinates resolution of both
    /// Bitcoin transactions and manifest hashes.
    ///
    /// # Arguments
    ///
    /// * `requests` - Collection of auxiliary data requests from pre-processing
    ///
    /// # Returns
    ///
    /// Returns `AuxData` containing:
    /// - Raw Bitcoin transaction data for each requested txid
    /// - Manifest hashes with MMR proofs for each requested height range
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any Bitcoin transaction cannot be fetched
    /// - Any historical manifest hash cannot be resolved
    /// - MMR proof generation fails
    pub fn resolve(&self, requests: &AuxRequests) -> WorkerResult<AuxData> {
        debug!(
            bitcoin_txs = requests.bitcoin_txs().len(),
            manifest_ranges = requests.manifest_hashes().len(),
            "Resolving auxiliary data requests"
        );

        // Resolve Bitcoin transactions
        let bitcoin_txs = self.resolve_bitcoin_txs(requests.bitcoin_txs())?;

        // Resolve manifest hashes with proofs
        let manifest_hashes = self.resolve_manifest_hashes(requests.manifest_hashes())?;

        debug!(
            resolved_txs = bitcoin_txs.len(),
            resolved_manifests = manifest_hashes.len(),
            "Successfully resolved auxiliary data"
        );

        Ok(AuxData::new(manifest_hashes, bitcoin_txs))
    }

    /// Resolves Bitcoin transactions by their txids.
    ///
    /// Fetches raw transaction data from the Bitcoin client for each requested txid
    /// using the `getrawtransaction` RPC. Requires the Bitcoin node to have transaction
    /// indexing enabled (`txindex=1` in bitcoin.conf).
    ///
    /// # Arguments
    ///
    /// * `txids` - List of Bitcoin transaction IDs to fetch
    ///
    /// # Returns
    ///
    /// Vector of raw Bitcoin transaction data in the same order as requested.
    ///
    /// # Errors
    ///
    /// Returns `WorkerError::BitcoinTxNotFound` if any transaction cannot be fetched.
    /// This can happen if:
    /// - The transaction does not exist
    /// - The Bitcoin node does not have txindex enabled
    /// - There's a network or RPC communication error
    fn resolve_bitcoin_txs(&self, txids: &[BitcoinTxid]) -> WorkerResult<Vec<RawBitcoinTx>> {
        if txids.is_empty() {
            return Ok(Vec::new());
        }

        debug!(count = txids.len(), "Resolving Bitcoin transactions");

        txids
            .iter()
            .map(|txid| {
                trace!(?txid, "Fetching Bitcoin transaction");
                self.context.get_bitcoin_tx(txid)
            })
            .collect()
    }

    /// Resolves historical manifest hashes with MMR proofs.
    ///
    /// For each height range, fetches the stored manifest hashes and generates
    /// MMR proofs using the AsmDBSled implementation.
    ///
    /// # Arguments
    ///
    /// * `ranges` - List of L1 block height ranges to resolve manifest hashes for
    ///
    /// # Returns
    ///
    /// Vector of manifest hashes with their MMR proofs.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Genesis height calculation fails
    /// - Any manifest hash cannot be fetched from storage
    /// - MMR proof generation fails
    /// - Requested height is before genesis
    fn resolve_manifest_hashes(
        &self,
        ranges: &[ManifestHashRange],
    ) -> WorkerResult<Vec<VerifiableManifestHash>> {
        if ranges.is_empty() {
            return Ok(Vec::new());
        }

        debug!(count = ranges.len(), "Resolving manifest hash ranges");

        let genesis_height = self
            .params
            .rollup()
            .genesis_l1_view
            .height()
            .to_consensus_u32() as u64;

        let mut resolved = Vec::new();

        for range in ranges {
            let start_height = range.start_height();
            let end_height = range.end_height();

            // Validate range is not before genesis
            if end_height < genesis_height {
                warn!(
                    start = start_height,
                    end = end_height,
                    genesis = genesis_height,
                    "Requested manifest hash range before genesis"
                );
                return Err(WorkerError::InvalidHeightRange {
                    start: start_height,
                    end: end_height,
                });
            }

            // Calculate MMR indices from L1 heights
            // MMR index 0 = genesis height, index 1 = genesis + 1, etc.
            let start_index = start_height.saturating_sub(genesis_height);
            let end_index = end_height - genesis_height;

            debug!(
                start_height,
                end_height, start_index, end_index, "Resolving manifest hash range"
            );

            for mmr_index in start_index..=end_index {
                // Fetch manifest hash from storage
                let manifest_hash: [u8; 32] = self
                    .context
                    .get_manifest_hash(mmr_index)?
                    .ok_or(WorkerError::ManifestHashNotFound { index: mmr_index })?;

                // Generate MMR proof for this index
                let proof_b32 = self.context.generate_mmr_proof(mmr_index)?;

                // Convert MerkleProofB32 to MerkleProof<Hash32> (AsmMerkleProof)
                // Both types contain the same data: index and cohashes
                // Extract from MerkleProofB32 and reconstruct as MerkleProof<Hash32>
                let cohashes: Vec<[u8; 32]> = proof_b32.cohashes();
                let index = proof_b32.index();
                let asm_proof = AsmMerkleProof::from_cohashes(cohashes, index);

                let hash = Hash32::from(manifest_hash);
                resolved.push(VerifiableManifestHash::new(hash, asm_proof));

                trace!(
                    index = mmr_index,
                    height = genesis_height + mmr_index,
                    "Resolved manifest hash with proof"
                );
            }
        }

        debug!(
            resolved_count = resolved.len(),
            "Successfully resolved manifest hashes with MMR proofs"
        );

        Ok(resolved)
    }
}

impl<'a> std::fmt::Debug for AuxDataResolver<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuxDataResolver")
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    // TODO: Add tests once we have mock implementations
    // - test_resolve_empty_requests
    // - test_resolve_bitcoin_txs
    // - test_resolve_manifest_hashes
    // - test_bitcoin_tx_not_found
    // - test_invalid_manifest_range
}
