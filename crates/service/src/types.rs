//! Core service worker types.

use std::{fmt::Debug, future::Future};

use serde::Serialize;

/// Response from handling an input.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Response {
    /// Normal case, should continue.
    Continue,

    /// Service should exit early.
    ShouldExit,
}

/// Abstract service trait.
pub trait Service: Sync + Send + 'static {
    /// The in-memory state of the service.
    type State: ServiceState;

    /// The input handle type, which lets us see the status type.
    // Make display?
    type Input: ServiceInput;

    /// The status type derived from the state.
    ///
    /// This implements [``Serialize``] so that we can unify different types of
    /// services into a single metrics collection system.
    type Status: Clone + Debug + Sync + Send + Serialize + 'static;

    /// Gets the status from the current state.
    fn get_status(s: &Self::State) -> Self::Status;
}

/// Trait for service states which exposes common properties.
pub trait ServiceState: Sync + Send + 'static {
    /// Name for a service that can be printed in logs.
    ///
    /// This SHOULD NOT change after the service worker has been started.
    fn name(&self) -> &str;
}

/// Trait for async service impls to define their per-input logic.
pub trait AsyncService: Service
where
    Self::Input: AsyncServiceInput,
{
    fn process_input(
        state: &mut Self::State,
        input: &<Self::Input as ServiceInput>::Msg,
    ) -> impl Future<Output = anyhow::Result<Response>> + Send;
}

/// Trait for blocking service impls to define their per-input logic.
pub trait SyncService: Service
where
    Self::Input: SyncServiceInput,
{
    fn process_input(
        state: &mut Self::State,
        input: &<Self::Input as ServiceInput>::Msg,
    ) -> anyhow::Result<Response>;
}

/// Generic service input trait.
pub trait ServiceInput: Sync + Send + 'static {
    /// The message type.
    type Msg: Sync + Send + Debug + 'static;
}

/// Common inputs for async service input sources.
pub trait AsyncServiceInput: ServiceInput {
    /// Receives the "next input".  If returns `Ok(None)` then there is no more
    /// input and we should exit.
    ///
    /// This is like a specialized `TryStream`.
    fn recv_next(&mut self) -> impl Future<Output = anyhow::Result<Option<Self::Msg>>> + Send;
}

/// Common inputs for blocking service input sources.
pub trait SyncServiceInput: ServiceInput {
    /// Receives the "next input".  If returns `Ok(None)` then there is no more
    /// input and we should exit.
    ///
    /// This is like a specialized `TryIterator`.
    fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>>;
}
