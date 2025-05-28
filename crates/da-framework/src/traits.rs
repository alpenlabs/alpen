//! Traits for working with DA

use std::io::Read;

/// Describes a way to change to a type.
pub trait DaWrite: Default {
    /// The target type we are applying the write to.
    type Target;

    /// Returns if this write is the default operation, like a no-op.
    fn is_default(&self) -> bool;

    /// Applies the write to the target type.
    fn apply(&self, target: &mut Self::Target);
}
