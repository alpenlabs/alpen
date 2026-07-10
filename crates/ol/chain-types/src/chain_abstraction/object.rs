//! Common inheritor trait.

use std::fmt::Debug;

use strata_identifiers::Buf32;

/// Common trait that other abstraction types inherit from.
///
/// This is mainly used to imply common std traits all at once.
pub trait IChainObj: Debug + Sync + Send + Sized {
    // nothing here yet
}
