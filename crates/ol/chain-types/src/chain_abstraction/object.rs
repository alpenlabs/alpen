//! Common inheritor trait.

use std::fmt::Debug;

/// Common trait that other abstraction types inherit from.
///
/// This is mainly used to imply common std traits all at once.
pub trait IChainObj: Debug + Sync + Send + Sized {
    // nothing here yet
}

/// Blanket impl for everything.
impl<T: Debug + Sync + Send + Sized> IChainObj for T {}
