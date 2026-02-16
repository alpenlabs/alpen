//! Auxiliary request collector.
//!
//! Collects auxiliary data requests from subprotocols during the pre-processing phase.

use bitcoin::Txid;

use crate::{
    aux::data::{AuxRequests, ManifestHashRange},
    logging,
};

/// Collects auxiliary data requests from subprotocols.
///
/// During `pre_process_txs`, subprotocols use this collector to register
/// their auxiliary data requirements (manifest hashes and Bitcoin transactions).
///
/// The collector is bounds-aware: it silently drops manifest hash requests for
/// heights beyond what is available in the accumulator. This prevents malicious
/// L1 transactions (e.g. a checkpoint claiming a beyond-tip L1 height) from
/// causing the aux resolver to fail, which would block processing of the entire
/// L1 block.
#[derive(Debug)]
pub struct AuxRequestCollector {
    requests: AuxRequests,
    min_manifest_height: u64,
    max_manifest_height: u64,
}

impl AuxRequestCollector {
    /// Creates a new empty collector bounded by the given manifest height range.
    pub fn new(min_manifest_height: u64, max_manifest_height: u64) -> Self {
        Self {
            requests: AuxRequests::default(),
            min_manifest_height,
            max_manifest_height,
        }
    }

    /// Requests manifest hashes for a block height range.
    ///
    /// # Arguments
    /// * `start_height` - Starting L1 block height (inclusive)
    /// * `end_height` - Ending L1 block height (inclusive)
    pub fn request_manifest_hashes(&mut self, start_height: u64, end_height: u64) {
        if start_height > end_height
            || start_height < self.min_manifest_height
            || end_height > self.max_manifest_height
        {
            logging::warn!(
                start_height,
                end_height,
                self.min_manifest_height,
                self.max_manifest_height,
                "dropping out-of-bounds manifest hash request"
            );
            return;
        }

        self.requests
            .manifest_hashes
            .push(ManifestHashRange::new(start_height, end_height));
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
        let mut collector = AuxRequestCollector::new(0, 500);
        assert!(collector.requests.manifest_hashes.is_empty());
        assert!(collector.requests.bitcoin_txs.is_empty());

        collector.request_manifest_hashes(100, 200);
        assert_eq!(collector.requests.manifest_hashes.len(), 1);

        collector.request_manifest_hashes(201, 300);
        assert_eq!(collector.requests.manifest_hashes.len(), 2);

        let requests = collector.into_requests();
        assert_eq!(requests.manifest_hashes.len(), 2);
        assert_eq!(requests.manifest_hashes[0].start_height(), 100);
        assert_eq!(requests.manifest_hashes[0].end_height(), 200);
        assert_eq!(requests.manifest_hashes[1].start_height(), 201);
        assert_eq!(requests.manifest_hashes[1].end_height(), 300);
    }

    #[test]
    fn test_collector_drops_beyond_max_height() {
        let mut collector = AuxRequestCollector::new(0, 200);

        collector.request_manifest_hashes(100, 200);
        assert_eq!(collector.requests.manifest_hashes.len(), 1);

        // end_height > max_manifest_height: silently dropped
        collector.request_manifest_hashes(100, 201);
        assert_eq!(collector.requests.manifest_hashes.len(), 1);
    }

    #[test]
    fn test_collector_drops_below_min_height() {
        let mut collector = AuxRequestCollector::new(100, 500);

        // start_height < min_manifest_height: dropped
        collector.request_manifest_hashes(50, 200);
        assert!(collector.requests.manifest_hashes.is_empty());

        // within bounds: accepted
        collector.request_manifest_hashes(100, 200);
        assert_eq!(collector.requests.manifest_hashes.len(), 1);
    }

    #[test]
    fn test_collector_drops_inverted_range() {
        let mut collector = AuxRequestCollector::new(0, 500);

        // start > end: silently dropped
        collector.request_manifest_hashes(200, 100);
        assert!(collector.requests.manifest_hashes.is_empty());
    }

    #[test]
    fn test_collector_bitcoin_tx() {
        let mut collector = AuxRequestCollector::new(0, 500);

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
