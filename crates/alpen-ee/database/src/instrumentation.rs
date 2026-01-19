//! Instrumentation component identifiers for EE database operations.

/// Component identifiers for tracing spans in EE database operations.
pub(crate) mod components {
    /// EENodeDatabase operations. Fields: account_id, blkid, finalized_height
    pub(crate) const STORAGE_EE_NODE: &str = "storage:ee_node";
}
