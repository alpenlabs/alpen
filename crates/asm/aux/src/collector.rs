//! Auxiliary request collector.
//!
//! Collects auxiliary data requests from subprotocols during the pre-processing phase.

use std::collections::BTreeMap;

use crate::{AuxRequestSpec, L1TxIndex};

/// Collects auxiliary data requests keyed by transaction index.
///
/// During `pre_process_txs`, subprotocols use this collector to register
/// their auxiliary data requirements. Each transaction can request at most
/// one auxiliary data item (identified by its index within the L1 block).
///
/// # Example
///
/// ```ignore
/// fn pre_process_txs(
///     state: &Self::State,
///     txs: &[TxInputRef],
///     collector: &mut AuxRequestCollector,
///     anchor_pre: &AnchorState,
///     params: &Self::Params,
/// ) {
///     for (idx, tx) in txs.iter().enumerate() {
///         // Request manifest leaves for blocks 100-200
///         collector.request(
///             idx,
///             AuxRequestSpec::manifest_leaves(100, 200),
///         );
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct AuxRequestCollector {
    /// Map from transaction index to its auxiliary request
    requests: BTreeMap<L1TxIndex, AuxRequestSpec>,
}

impl AuxRequestCollector {
    /// Creates a new empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an auxiliary data request for a specific transaction.
    ///
    /// # Arguments
    ///
    /// * `tx_index` - Index of the transaction within the L1 block (0-based)
    /// * `spec` - Specification of what auxiliary data is needed
    ///
    /// # Panics
    ///
    /// Panics if a request was already registered for this transaction index.
    /// Each transaction can only request one auxiliary data item.
    pub fn request(&mut self, tx_index: L1TxIndex, spec: AuxRequestSpec) {
        if self.requests.insert(tx_index, spec).is_some() {
            panic!(
                "duplicate auxiliary request for transaction index {}",
                tx_index
            );
        }
    }

    /// Consumes the collector and returns all collected requests.
    ///
    /// This is typically called by the orchestration layer after all
    /// subprotocols have completed their pre-processing phase.
    pub fn into_requests(self) -> BTreeMap<L1TxIndex, AuxRequestSpec> {
        self.requests
    }

    /// Returns the number of pending auxiliary requests.
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    /// Returns true if no auxiliary requests have been collected.
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Returns a reference to the requests map.
    pub fn requests(&self) -> &BTreeMap<L1TxIndex, AuxRequestSpec> {
        &self.requests
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collector_basic() {
        let mut collector = AuxRequestCollector::new();
        assert!(collector.is_empty());
        assert_eq!(collector.len(), 0);

        collector.request(0, AuxRequestSpec::manifest_leaves(100, 200));
        assert_eq!(collector.len(), 1);
        assert!(!collector.is_empty());

        collector.request(1, AuxRequestSpec::manifest_leaves(201, 300));
        assert_eq!(collector.len(), 2);

        let requests = collector.into_requests();
        assert_eq!(requests.len(), 2);
        assert!(requests.contains_key(&0));
        assert!(requests.contains_key(&1));
    }

    #[test]
    #[should_panic(expected = "duplicate auxiliary request")]
    fn test_collector_duplicate_panics() {
        let mut collector = AuxRequestCollector::new();
        collector.request(0, AuxRequestSpec::manifest_leaves(100, 200));
        // This should panic
        collector.request(0, AuxRequestSpec::manifest_leaves(201, 300));
    }
}
