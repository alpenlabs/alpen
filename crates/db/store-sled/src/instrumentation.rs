//! Instrumentation component identifiers for sled database operations.

/// Component identifiers for tracing spans in sled database operations.
pub(crate) mod components {
    /// Sled transaction lifecycle. Fields: tx_id, attempt, conflict_key. DEBUG level only.
    pub(crate) const DB_SLED_TRANSACTION: &str = "db:sled:transaction";
}
