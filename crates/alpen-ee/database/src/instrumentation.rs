//! Instrumentation component identifiers for EE database operations.

/// Component identifiers for tracing spans in EE database operations.
///
/// The per-op spans are emitted by the `gen_proxy`-generated `EeNodeDb` proxy,
/// whose `tracing_component` attribute mirrors this exact value. The constant
/// is retained as the canonical registry even though the proxy references the
/// string literal directly.
#[allow(dead_code, reason = "mirrored into gen_proxy `tracing_component` attribute")]
pub(crate) mod components {
    /// EENodeDatabase operations. Fields: account_id, blkid, finalized_height
    pub(crate) const STORAGE_EE_NODE: &str = "storage:ee_node";
}
