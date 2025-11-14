//! Auxiliary request collector.
//!
//! Collects auxiliary data requests from subprotocols during the pre-processing phase.

use bitcoin::Txid;

use crate::aux::data::{AuxRequests, ManifestHashRange};

/// Collects auxiliary data requests from subprotocols.
///
/// During `pre_process_txs`, subprotocols use this collector to register
/// their auxiliary data requirements (manifest hashes and Bitcoin transactions).
#[derive(Debug, Default)]
pub struct AuxRequestCollector {
    requests: AuxRequests,
}

impl AuxRequestCollector {
    /// Creates a new empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests manifest hashes for a block height range.
    ///
    /// # Arguments
    /// * `start_height` - Starting L1 block height (inclusive)
    /// * `end_height` - Ending L1 block height (inclusive)
    pub fn request_manifest_hashes(&mut self, start_height: u64, end_height: u64) {
        self.requests.manifest_hashes.push(ManifestHashRange {
            start_height,
            end_height,
        });
    }

    /// Requests a raw Bitcoin transaction by its txid.
    pub fn request_bitcoin_tx(&mut self, txid: Txid) {
        self.requests.bitcoin_txs.push(txid.into());
    }

    /// Consumes the collector and returns the collected auxiliary requests.
    pub fn into_requests(self) -> AuxRequests {
        self.requests
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;

    use super::*;

    #[test]
    fn test_collector_basic() {
        let mut collector = AuxRequestCollector::new();
        assert!(collector.requests.manifest_hashes.is_empty());
        assert!(collector.requests.bitcoin_txs.is_empty());

        collector.request_manifest_hashes(100, 200);
        assert_eq!(collector.requests.manifest_hashes.len(), 1);

        collector.request_manifest_hashes(201, 300);
        assert_eq!(collector.requests.manifest_hashes.len(), 2);

        let requests = collector.into_requests();
        assert_eq!(requests.manifest_hashes.len(), 2);
        assert_eq!(requests.manifest_hashes[0].start_height, 100);
        assert_eq!(requests.manifest_hashes[0].end_height, 200);
        assert_eq!(requests.manifest_hashes[1].start_height, 201);
        assert_eq!(requests.manifest_hashes[1].end_height, 300);
    }

    #[test]
    fn test_collector_bitcoin_tx() {
        let mut collector = AuxRequestCollector::new();

        let txid1 = Txid::from_byte_array([1u8; 32]);
        let txid2 = Txid::from_byte_array([2u8; 32]);
        collector.request_bitcoin_tx(txid1);
        collector.request_bitcoin_tx(txid2);

        assert_eq!(collector.requests.bitcoin_txs.len(), 2);

        let requests = collector.into_requests();
        assert_eq!(requests.bitcoin_txs[0], txid1.into());
        assert_eq!(requests.bitcoin_txs[1], txid2.into());
    }
}
