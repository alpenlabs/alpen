use strata_msg_fmt::TypeId;

/// Macro to define all type IDs and ensure they're included in uniqueness tests
macro_rules! define_type_ids {
    ($($name:ident = $value:expr),* $(,)?) => {
        $(
            pub const $name: TypeId = $value;
        )*

        /// Get all defined type IDs as an array
        pub const fn all_type_ids() -> &'static [TypeId] {
            &[$($name),*]
        }
    };
}

define_type_ids! {
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
        let type_ids = all_type_ids();
        let unique_ids: HashSet<_> = type_ids.iter().collect();
        assert_eq!(
            type_ids.len(),
            unique_ids.len(),
            "All type IDs must be unique"
        );
    }
}
