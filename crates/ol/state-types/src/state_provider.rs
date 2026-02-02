//! State provider trait for fetching OL state at specific chain tips.
//!
//! This module provides the [`StateProvider`] trait which abstracts how components
//! retrieve OL chain state. This enables:
//! - Production use with [`OLStateManager`] (database-backed)
//! - Fast in-memory testing without database infrastructure
//! - Easy mocking for edge cases and error scenarios

use std::{error::Error, fmt::Debug, future::Future, sync::Arc};

use strata_identifiers::OLBlockCommitment;
use strata_ledger_types::IStateAccessor;

/// Provider trait for retrieving state at specific chain tips.
///
/// This trait abstracts the source of state data, allowing components like mempool
/// and block assembly to work with both database-backed storage (production) and
/// in-memory storage (testing).
///
/// # Associated Types
///
/// - `State`: The state type that implements [`IStateAccessor`]. Must be `Send + Sync + 'static` to
///   enable sharing across async operations.
/// - `Error`: The error type for state retrieval operations. Each implementation can use its own
///   error type (e.g., `DbError` for database-backed providers, custom errors for test providers).
///
/// # Examples
///
/// Implementing for a custom provider:
///
/// ```rust,ignore
/// use std::{collections::HashMap, sync::Arc};
/// use strata_identifiers::OLBlockCommitment;
/// use strata_ol_state_types::{OLState, StateProvider};
///
/// #[derive(Debug, thiserror::Error)]
/// enum MyError {
///     #[error("state not found")]
///     NotFound,
/// }
///
/// struct MyStateProvider {
///     states: HashMap<OLBlockCommitment, Arc<OLState>>,
/// }
///
/// impl StateProvider for MyStateProvider {
///     type State = OLState;
///     type Error = MyError;
///
///     async fn get_state_for_tip_async(
///         &self,
///         tip: OLBlockCommitment,
///     ) -> Result<Option<Arc<Self::State>>, Self::Error> {
///         Ok(self.states.get(&tip).cloned())
///     }
///
///     fn get_state_for_tip_blocking(
///         &self,
///         tip: OLBlockCommitment,
///     ) -> Result<Option<Arc<Self::State>>, Self::Error> {
///         Ok(self.states.get(&tip).cloned())
///     }
/// }
/// ```
pub trait StateProvider: Send + Sync + 'static {
    /// The state type that implements [`IStateAccessor`].
    ///
    /// Must be owned and Arc-able for sharing across validation and execution operations.
    type State: IStateAccessor + Send + Sync + Debug + 'static;

    /// Error type for state retrieval operations.
    ///
    /// Each implementation can define its own error type to provide appropriate context.
    type Error: Error + Send + Sync + 'static;

    /// Retrieves the state for a given chain tip asynchronously.
    ///
    /// Returns `None` if no state exists for the given tip.
    ///
    /// # Errors
    ///
    /// Returns an error if state retrieval fails (e.g., database error,
    /// network error, etc.). The specific error type depends on the
    /// implementation.
    fn get_state_for_tip_async(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<Arc<Self::State>>, Self::Error>> + Send;

    /// Retrieves the state for a given chain tip in a blocking manner.
    ///
    /// Returns `None` if no state exists for the given tip.
    ///
    /// # Errors
    ///
    /// Returns an error if state retrieval fails (e.g., database error,
    /// network error, etc.). The specific error type depends on the
    /// implementation.
    fn get_state_for_tip_blocking(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Arc<Self::State>>, Self::Error>;
}

/// Blanket implementation for Arc-wrapped state providers.
///
/// Enables sharing state providers across async boundaries without
/// requiring the inner type to implement Clone.
impl<T: StateProvider> StateProvider for Arc<T> {
    type State = T::State;
    type Error = T::Error;

    fn get_state_for_tip_async(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<Arc<Self::State>>, Self::Error>> + Send {
        T::get_state_for_tip_async(self, tip)
    }

    fn get_state_for_tip_blocking(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Arc<Self::State>>, Self::Error> {
        T::get_state_for_tip_blocking(self, tip)
    }
}
