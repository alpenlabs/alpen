//! Auxiliary request collector.
//!
//! Collects auxiliary data requests from subprotocols during the pre-processing phase.

use strata_identifiers::Buf32;

use crate::aux::data::AuxRequests;

/// Collects auxiliary data requests from subprotocols.
///
/// During `pre_process_txs`, subprotocols use this collector to register
/// their auxiliary data requirements (manifest leaves and Bitcoin transactions).
#[derive(Debug, Default)]
pub struct AuxRequestCollector {
    requests: AuxRequests,
}

impl AuxRequestCollector {
    /// Creates a new empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests manifest leaves for a block height range.
    ///
    /// # Arguments
    /// * `start_height` - Starting L1 block height (inclusive)
    /// * `end_height` - Ending L1 block height (inclusive)
    pub fn request_manifest_leaves(&mut self, start_height: u64, end_height: u64) {
        self.requests
            .manifest_leaves
            .push((start_height, end_height));
    }

    /// Requests a raw Bitcoin transaction by its txid.
    ///
    /// # Arguments
    /// * `txid` - The Bitcoin transaction ID (32 bytes)
    pub fn request_bitcoin_tx(&mut self, txid: Buf32) {
        self.requests.bitcoin_txs.push(txid);
    }

    /// Consumes the collector and returns the collected auxiliary requests.
    pub fn into_requests(self) -> AuxRequests {
        self.requests
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collector_basic() {
        let mut collector = AuxRequestCollector::new();
        assert!(collector.requests.manifest_leaves.is_empty());
        assert!(collector.requests.bitcoin_txs.is_empty());

        collector.request_manifest_leaves(100, 200);
        assert_eq!(collector.requests.manifest_leaves.len(), 1);

        collector.request_manifest_leaves(201, 300);
        assert_eq!(collector.requests.manifest_leaves.len(), 2);

        let requests = collector.into_requests();
        assert_eq!(requests.manifest_leaves.len(), 2);
        assert_eq!(requests.manifest_leaves[0], (100, 200));
        assert_eq!(requests.manifest_leaves[1], (201, 300));
    }

    #[test]
    fn test_collector_bitcoin_tx() {
        let mut collector = AuxRequestCollector::new();

        collector.request_bitcoin_tx([1u8; 32].into());
        collector.request_bitcoin_tx([2u8; 32].into());

        assert_eq!(collector.requests.bitcoin_txs.len(), 2);

        let requests = collector.into_requests();
        assert_eq!(requests.bitcoin_txs[0], [1u8; 32].into());
        assert_eq!(requests.bitcoin_txs[1], [2u8; 32].into());
    }
}
