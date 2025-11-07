//! Auxiliary request collector.
//!
//! Collects auxiliary data requests from subprotocols during the pre-processing phase.

use crate::{BitcoinTxRequest, L1TxIndex, ManifestLeavesRequest, aux::data::AuxRequests};

/// Collects auxiliary data requests keyed by transaction index.
///
/// During `pre_process_txs`, subprotocols use this collector to register
/// their auxiliary data requirements. Each transaction can request at most
/// one item per request type (e.g., one `ManifestLeavesRequest` and one
/// `BitcoinTxRequest`).
#[derive(Debug, Default)]
pub struct AuxRequestCollector {
    requests: AuxRequests,
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
    ///
    /// # Panics
    ///
    /// Panics if a manifest leaves request already exists for this transaction index.
    pub fn request_manifest_leaves(&mut self, tx_index: L1TxIndex, req: ManifestLeavesRequest) {
        if self
            .requests
            .manifest_leaves
            .insert(tx_index, req)
            .is_some()
        {
            panic!(
                "duplicate auxiliary request for transaction index {}",
                tx_index
            );
        }
    }

    /// Requests a raw Bitcoin transaction by its txid.
    ///
    /// # Panics
    ///
    /// Panics if a Bitcoin transaction request already exists for this transaction index.
    pub fn request_bitcoin_tx(&mut self, tx_index: L1TxIndex, req: BitcoinTxRequest) {
        if self.requests.bitcoin_txs.insert(tx_index, req).is_some() {
            panic!(
                "duplicate auxiliary request for transaction index {}",
                tx_index
            );
        }
    }

    /// Consumes the collector and returns the collected auxiliary requests.
    pub fn into_requests(self) -> AuxRequests {
        self.requests
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AsmCompactMmr, AsmMmr};

    #[test]
    fn test_collector_basic() {
        let mut collector = AuxRequestCollector::new();
        assert!(collector.requests.manifest_leaves.is_empty());
        assert!(collector.requests.bitcoin_txs.is_empty());

        let mmr = AsmMmr::new(16);
        let mmr_compact: AsmCompactMmr = mmr.into();
        let req0 = ManifestLeavesRequest {
            start_height: 100,
            end_height: 200,
            manifest_mmr: mmr_compact.clone(),
        };
        collector.request_manifest_leaves(0, req0);
        assert_eq!(collector.requests.manifest_leaves.len(), 1);

        let req1 = ManifestLeavesRequest {
            start_height: 201,
            end_height: 300,
            manifest_mmr: mmr_compact,
        };
        collector.request_manifest_leaves(1, req1);
        assert_eq!(collector.requests.manifest_leaves.len(), 2);

        assert!(collector.requests.manifest_leaves.contains_key(&0));
        assert!(collector.requests.manifest_leaves.contains_key(&1));
    }

    #[test]
    #[should_panic(expected = "duplicate auxiliary request")]
    fn test_collector_duplicate_panics() {
        let mut collector = AuxRequestCollector::new();
        let mmr = AsmMmr::new(16);
        let mmr_compact: AsmCompactMmr = mmr.into();
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
