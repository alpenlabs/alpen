//! Core service worker types.

use std::{any::Any, fmt::Debug, future::Future};

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

    /// The input message type that the service operates on.
    type Msg: ServiceMsg;

    /// The status type derived from the state.
    type Status: ServiceStatus;

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

/// Trait for service messages, which we want to treat like simple dumb data
/// containers.
///
/// This is also `Debug` for debug purposes.
pub trait ServiceMsg: Debug + Sync + Send + 'static {
    // nothing yet
}

/// Blanket auto-impl for any type that impls these traits.
impl<T: Debug + Sync + Send + 'static> ServiceMsg for T {}

/// Trait for service status.
///
/// This implements [``Serialize``] so that we can unify different types of
/// services into a single metrics collection system.
pub trait ServiceStatus: Any + Clone + Debug + Sync + Send + Serialize + 'static {
    // nothing yet
}

/// Blanket auto-impl for any type that impls these traits.
impl<T: Any + Clone + Debug + Sync + Send + Serialize + 'static> ServiceStatus for T {}

/// Trait for async service impls to define their per-input logic.
pub trait AsyncService: Service {
    /// Called in the worker task after launching.
    fn on_launch(_state: &mut Self::State) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }

    /// Called for each input.
    fn process_input(
        _state: &mut Self::State,
        _input: &Self::Msg,
    ) -> impl Future<Output = anyhow::Result<Response>> + Send {
        async { Ok(Response::Continue) }
    }

    /// Called when about to shut down, for whatever reason.
    ///
    /// Passed an error, if shutting down due to input handling error.
    fn before_shutdown(
        _state: &mut Self::State,
        _err: Option<&anyhow::Error>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }
}

/// Trait for blocking service impls to define their per-input logic.
pub trait SyncService: Service {
    /// Called in the worker thread after launching.
    fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    /// Called for each input.
    fn process_input(_state: &mut Self::State, _input: &Self::Msg) -> anyhow::Result<Response> {
        Ok(Response::Continue)
    }

    /// Called when about to shut down, for whatever reason.
    ///
    /// Passed an error, if shutting down due to input handling error.
    fn before_shutdown(
        _state: &mut Self::State,
        _err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Generic service input trait.
pub trait ServiceInput: Sync + Send + 'static {
    /// The message type.
    type Msg: ServiceMsg;
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
