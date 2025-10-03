//! Traits for working with DA

use crate::{BuilderError, DaError};

/// Describes a way to change to a type.
pub trait DaWrite: Default {
    /// The target type we are applying the write to.
    type Target;

    /// Context type we can provide additional inputs to.
    ///
    /// Default is nothing.
    type Context;

    /// Returns if this write is the default operation, like a no-op.
    fn is_default(&self) -> bool;

    /// Polls the context impl with the queries that would be made if we were
    /// really applying the DA, but without making any changes.
    fn poll_context(&self, _target: &Self::Target, _context: &Self::Context) -> Result<(), DaError> {
        // do nothing by default
        Ok(())
    }

    /// Applies the write to the target type.
    fn apply(&self, target: &mut Self::Target, context: &Self::Context) -> Result<(), DaError>;
}

/// Extension trait for when a [`DaWrite`] uses an empty context.
pub trait ContextlessDaWrite: DaWrite<Context = ()> {
    fn apply(&self, target: &mut <Self as DaWrite>::Target) -> Result<(), DaError>;
}

impl<W: DaWrite<Context = ()>> ContextlessDaWrite for W {
    fn apply(&self, target: &mut <Self as DaWrite>::Target) -> Result<(), DaError> {
        <Self as DaWrite>::apply(self, target, &())
    }
}

/// Abstract DA write builder.
pub trait DaBuilder<T> {
    /// Write type that will be generated when the builder is finalized.
    type Write;

    /// Constructs a builder from the source type.
    fn from_source(t: T) -> Self;

    /// Finalizes the write being generated.
    fn into_write(self) -> Result<Self::Write, BuilderError>;
}
