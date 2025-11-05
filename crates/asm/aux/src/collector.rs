//! Auxiliary request collector.
//!
//! Collects auxiliary data requests from subprotocols during the pre-processing phase.

use std::collections::BTreeMap;

use crate::{BitcoinTxRequest, L1TxIndex, ManifestLeavesRequest};

/// Collects auxiliary data requests keyed by transaction index.
///
/// During `pre_process_txs`, subprotocols use this collector to register
/// their auxiliary data requirements. Each transaction can request at most
/// one item per request type (e.g., one `ManifestLeavesRequest` and one
/// `BitcoinTxRequest`).
#[derive(Debug, Default)]
pub struct AuxRequestCollector {
    manifest_leaves: BTreeMap<L1TxIndex, ManifestLeavesRequest>,
    bitcoin_txs: BTreeMap<L1TxIndex, BitcoinTxRequest>,
}

impl AuxRequestCollector {
    /// Creates a new empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests manifest leaves (hash + proof) for a block height range.
    ///
    /// Stores a request keyed by `tx_index` containing the range and the
    /// compact manifest MMR snapshot used for verification by the provider.
    pub fn request_manifest_leaves(&mut self, tx_index: L1TxIndex, req: ManifestLeavesRequest) {
        // Use the common insertion logic to enforce one-request-per-tx
        if self.manifest_leaves.insert(tx_index, req).is_some() {
            panic!(
                "duplicate auxiliary request for transaction index {}",
                tx_index
            );
        }
    }

    /// Requests a raw Bitcoin transaction by its txid.
    pub fn request_bitcoin_tx(&mut self, tx_index: L1TxIndex, req: BitcoinTxRequest) {
        if self.bitcoin_txs.insert(tx_index, req).is_some() {
            panic!(
                "duplicate auxiliary request for transaction index {}",
                tx_index
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_common::AsmManifestCompactMmr;

    use super::*;

    #[test]
    fn test_collector_basic() {
        let mut collector = AuxRequestCollector::new();
        assert!(collector.manifest_leaves.is_empty());
        assert!(collector.bitcoin_txs.is_empty());

        let mmr = strata_asm_common::AsmManifestMmr::new(16);
        let mmr_compact: AsmManifestCompactMmr = mmr.into();
        let req0 = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: mmr_compact.clone(),
        };
        collector.request_manifest_leaves(0, req0);
        assert_eq!(collector.manifest_leaves.len(), 1);

        let req1 = ManifestLeavesRequest {
            start_height: 201,
            end_height: 300,
            manifest_mmr: mmr_compact,
        };
        collector.request_manifest_leaves(1, req1);
        assert_eq!(collector.manifest_leaves.len(), 2);

        assert!(collector.manifest_leaves.contains_key(&0));
        assert!(collector.manifest_leaves.contains_key(&1));
    }

    #[test]
    #[should_panic(expected = "duplicate auxiliary request")]
    fn test_collector_duplicate_panics() {
        let mut collector = AuxRequestCollector::new();
        let mmr = strata_asm_common::AsmManifestMmr::new(16);
        let mmr_compact: AsmManifestCompactMmr = mmr.into();
        let req0 = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: mmr_compact.clone(),
        };
        collector.request_manifest_leaves(0, req0);
        // This should panic
        let req1 = ManifestLeavesRequest {
            start_height: 201,
            end_height: 300,
            manifest_mmr: mmr_compact,
        };
        collector.request_manifest_leaves(0, req1);
    }
}
