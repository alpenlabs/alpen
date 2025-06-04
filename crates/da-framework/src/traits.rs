//! Traits for working with DA

use crate::BuilderError;

/// Describes a way to change to a type.
pub trait DaWrite: Default {
    /// The target type we are applying the write to.
    type Target;

    /// Returns if this write is the default operation, like a no-op.
    fn is_default(&self) -> bool;

    /// Applies the write to the target type.
    fn apply(&self, target: &mut Self::Target);
}

pub trait DaBuilder<T> {
    /// Write type that will be generated when the builder is finalized.
    type Write;

    /// Constructs a builder from the source type.
    fn from_source(t: T) -> Self;

    /// Finalizes the write being generated.
    fn into_write(self) -> Result<Self::Write, BuilderError>;
}
