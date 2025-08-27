//! Traits for output formatting

use serde::Serialize;

/// Trait for objects that can be formatted for porcelain output
pub(crate) trait Formattable {
    /// Format for machine-readable output (parseable, stable, human-readable)
    fn format_porcelain(&self) -> String;
}

// TODO(QQ): remove, this is stub to ensure compilation

#[derive(Serialize)]
pub(crate) struct FmtStub {}
impl Formattable for FmtStub {
    fn format_porcelain(&self) -> String {
        "".to_string()
    }
}
