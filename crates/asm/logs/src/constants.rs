use strata_msg_fmt::TypeId;

/// Macro to define all type IDs and ensure they're included in uniqueness tests
macro_rules! define_ids {
    ($type:ty, $const_name:ident, $($name:ident = $value:expr),* $(,)?) => {
        $(
            pub const $name: $type = $value;
        )*

        /// Array containing all defined type IDs
        pub const $const_name: &'static [$type] = &[$($name),*];
    };
}

// Define all log type IDs
define_ids! {TypeId, LOG_TYPE_IDS,
    DEPOSIT_LOG_TYPE_ID = 1,
    FORCED_INCLUSION_LOG_TYPE_ID = 2,
    CHECKPOINT_UPDATE_LOG_TYPE_ID = 3,
    OL_STF_UPDATE_LOG_TYPE_ID = 4,
    ASM_STF_UPDATE_LOG_TYPE_ID = 5,
    NEW_EXPORT_ENTRY_LOG_TYPE_ID = 6,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn test_all_type_ids_are_unique() {
        let log_ids = LOG_TYPE_IDS;
        let unique_ids: HashSet<_> = log_ids.iter().collect();
        assert_eq!(
            log_ids.len(),
            unique_ids.len(),
            "All type IDs must be unique"
        );
    }
}
