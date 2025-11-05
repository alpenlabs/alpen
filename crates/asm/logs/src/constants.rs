use int_enum::IntEnum;

/// Log type identifiers for ASM logs
#[repr(u16)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, IntEnum)]
pub enum LogTypeId {
    Deposit = 1,
    ForcedInclusion = 2,
    CheckpointUpdate = 3,
    OlStfUpdate = 4,
    AsmStfUpdate = 5,
    NewExportEntry = 6,
}

impl LogTypeId {
    pub fn from_type_id_raw(type_id: u16) -> Option<Self> {
        Self::try_from(type_id).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn test_all_type_ids_are_unique() {
        // Collect all enum variants
        let log_ids = [
            LogTypeId::Deposit,
            LogTypeId::ForcedInclusion,
            LogTypeId::CheckpointUpdate,
            LogTypeId::OlStfUpdate,
            LogTypeId::AsmStfUpdate,
            LogTypeId::NewExportEntry,
        ];

        // Convert to u16 values for uniqueness check
        let id_values: Vec<u16> = log_ids.iter().map(|&id| id as u16).collect();
        let unique_ids: HashSet<_> = id_values.iter().collect();

        assert_eq!(
            id_values.len(),
            unique_ids.len(),
            "All type IDs must be unique"
        );
    }

    #[test]
    fn test_int_enum_conversion() {
        // Test that we can convert between enum and u16
        assert_eq!(LogTypeId::Deposit as u16, 1);
        assert_eq!(LogTypeId::ForcedInclusion as u16, 2);
        assert_eq!(LogTypeId::CheckpointUpdate as u16, 3);
        assert_eq!(LogTypeId::OlStfUpdate as u16, 4);
        assert_eq!(LogTypeId::AsmStfUpdate as u16, 5);
        assert_eq!(LogTypeId::NewExportEntry as u16, 6);

        // Test IntEnum trait methods
        assert_eq!(LogTypeId::try_from(1), Ok(LogTypeId::Deposit));
        assert_eq!(LogTypeId::try_from(2), Ok(LogTypeId::ForcedInclusion));
        assert!(LogTypeId::try_from(99).is_err());
    }
}
