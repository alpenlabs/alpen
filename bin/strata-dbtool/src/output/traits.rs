//! Traits for output formatting

/// Trait for objects that can be formatted for porcelain output
pub(crate) trait Formattable {
    /// Format for machine-readable output (parseable, stable, human-readable)
    fn format_porcelain(&self) -> String;
}
