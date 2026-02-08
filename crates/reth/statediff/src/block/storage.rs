//! Per-block storage diff types.

use std::collections::BTreeMap;

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};

/// Per-account storage diff with original values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockStorageDiff {
    /// Storage slot changes: slot_key -> (original_value, current_value).
    pub slots: BTreeMap<U256, (U256, U256)>,
}

impl BlockStorageDiff {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_storage_diff_roundtrip() {
        let mut diff = BlockStorageDiff::new();
        diff.slots
            .insert(U256::from(1), (U256::from(0), U256::from(100)));
        diff.slots
            .insert(U256::from(2), (U256::from(50), U256::from(0)));

        let encoded = serde_json::to_vec(&diff).unwrap();
        let decoded: BlockStorageDiff = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded, diff);
    }
}
