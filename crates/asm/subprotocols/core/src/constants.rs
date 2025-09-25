use strata_l1_txfmt::{SubprotocolId, TxType};

/// Macro to define all type IDs and ensure they're included in uniqueness tests
macro_rules! define_ids {
    ($type:ty, $const_name:ident, $($name:ident = $value:expr),* $(,)?) => {
        $(
            pub(crate) const $name: $type = $value;
        )*

        /// Array containing all defined type IDs
        #[allow(dead_code, clippy::allow_attributes, reason = "some generated code is not used")]
        const $const_name: &'static [$type] = &[$($name),*];
    };
}

// Define all subprotocol IDs
define_ids! {SubprotocolId, SUBPROTOCOL_IDS,
    CORE_SUBPROTOCOL_ID = 1,
    BRIDGE_SUBPROTOCOL_ID = 2,
}

// Define all transaction type IDs
// [PLACE_HOLDER] TODO: Export TxType definitions from strata-common repo instead of defining
// locally
define_ids! {TxType, CORE_TX_TYPE_IDS,
    OL_STF_CHECKPOINT_TX_TYPE = 1,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn test_all_type_ids_are_unique() {
        let subprotocol_ids = SUBPROTOCOL_IDS;
        let unique_ids: HashSet<_> = subprotocol_ids.iter().collect();
        assert_eq!(
            subprotocol_ids.len(),
            unique_ids.len(),
            "All subprotocol IDs must be unique"
        );

        let tx_type_ids = CORE_TX_TYPE_IDS;
        let unique_ids: HashSet<_> = tx_type_ids.iter().collect();
        assert_eq!(
            tx_type_ids.len(),
            unique_ids.len(),
            "All transaction type IDs must be unique"
        );
    }
}
