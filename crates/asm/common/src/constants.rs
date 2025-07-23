use strata_l1_txfmt::{SubprotocolId, TxType};
use strata_msg_fmt::TypeId;

/// Macro to define all type IDs and ensure they're included in uniqueness tests
macro_rules! define_ids {
    ($type:ty, $fn_name:ident, $($name:ident = $value:expr),* $(,)?) => {
        $(
            pub const $name: $type = $value;
        )*

        /// Get all defined type IDs as an array
        pub const fn $fn_name() -> &'static [$type] {
            &[$($name),*]
        }
    };
}

// Define all subprotocol IDs
define_ids! {SubprotocolId, all_subprotocol_ids,
    CORE_SUBPROTOCOL_ID = 1,
    BRIDGE_SUBPROTOCOL_ID = 2,
}

// Define all transaction type IDs
define_ids! {TxType, all_core_tx_type_ids,
    OL_STF_CHECKPOINT_TX_TYPE = 1,
    FORCED_INCLUSION_TX_TYPE = 2,
    EE_UPGRADE_TX_TYPE = 3,
}

// Define all log type IDs
define_ids! {TypeId, all_log_type_ids,
    DEPOSIT_LOG_TYPE_ID = 1,
    FORCED_INCLUSION_LOG_TYPE_ID = 2,
    CHECKPOINT_UPDATE_LOG_TYPE = 3,
    OL_STF_UPDATE_LOG_TYPE = 4,
    ASM_STF_UPDATE_LOG_TYPE = 5,
    NEW_EXPORT_ENTRY_LOG_TYPE = 6,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn test_all_type_ids_are_unique() {
        let subprotocol_ids = all_subprotocol_ids();
        let unique_ids: HashSet<_> = subprotocol_ids.iter().collect();
        assert_eq!(
            subprotocol_ids.len(),
            unique_ids.len(),
            "All subprotocol IDs must be unique"
        );

        let tx_type_ids = all_core_tx_type_ids();
        let unique_ids: HashSet<_> = tx_type_ids.iter().collect();
        assert_eq!(
            tx_type_ids.len(),
            unique_ids.len(),
            "All transaction type IDs must be unique"
        );

        let log_ids = all_log_type_ids();
        let unique_ids: HashSet<_> = log_ids.iter().collect();
        assert_eq!(
            log_ids.len(),
            unique_ids.len(),
            "All type IDs must be unique"
        );
    }
}
