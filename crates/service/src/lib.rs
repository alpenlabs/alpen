//! Service framework modules.
//!
//! This crate provides a structured framework for building worker tasks that process
//! messages with lifecycle management, status monitoring, and graceful shutdown.
//!
//! # Service Patterns
//!
//! ## Command Worker Pattern
//!
//! Command workers are passive services that respond to explicit commands via a handle.
//! They use MPSC channels for communication and can optionally return responses via
//! oneshot channels.
//!
//! ```rust,ignore
//! use strata_service::*;
//!
//! // Define command messages
//! enum MyCommand {
//!     DoWork(u32, CommandCompletionSender<String>),
//! }
//!
//! // Implement the service
//! impl Service for MyService {
//!     type State = MyState;
//!     type Msg = MyCommand;
//!     type Status = MyStatus;
//!     // ...
//! }
//!
//! // Create a command handle
//! let mut builder = ServiceBuilder::<MyService, _>::new();
//! let cmd_handle = builder.create_command_handle(10);
//! let monitor = builder
//!     .with_state(state)
//!     .launch_async("my_service", &texec)
//!     .await?;
//!
//! // Use the handle to send commands
//! let result = cmd_handle.send_and_wait(|completion| {
//!     MyCommand::DoWork(42, completion)
//! }).await?;
//! ```
//!
//! ## Listener Pattern
//!
//! Listeners are passive services that react to status updates from another service.
//! The monitored service is unaware of the listener's existence. When the monitored
//! service exits (watch channel closes), the listener automatically exits.
//!
//! ```rust,ignore
//! use strata_service::*;
//!
//! // Launch a service to monitor
//! let monitored_monitor = ServiceBuilder::<MonitoredService, _>::new()
//!     .with_state(monitored_state)
//!     .with_input(monitored_input)
//!     .launch_async("monitored", &texec)
//!     .await?;
//!
//! // Create a listener that reacts to status changes
//! let listener_input = StatusMonitorInput::from_receiver(
//!     monitored_monitor.status_rx.clone()
//! );
//!
//! let listener_monitor = ServiceBuilder::<ListenerService, _>::new()
//!     .with_state(listener_state)
//!     .with_input(listener_input)
//!     .launch_async("listener", &texec)
//!     .await?;
//!
//! // ListenerService receives MonitoredService::Status as messages
//! impl AsyncService for ListenerService {
//!     async fn process_input(
//!         state: &mut Self::State,
//!         status: &MonitoredStatus,
//!     ) -> anyhow::Result<Response> {
//!         // React to status changes
//!         println!("Monitored service status: {:?}", status);
//!         Ok(Response::Continue)
//!     }
//! }
//! ```
//!
//! Key properties of the listener pattern:
//! - **Passive**: Only reacts to status updates, doesn't actively poll
//! - **Unaware**: Monitored service has no knowledge of listeners
//! - **Coupled lifecycle**: Listener exits when monitored service exits
//! - **Own status**: Listener maintains its own status structure
//! - **Efficient**: Uses Tokio's watch channel for minimal overhead

mod adapters;
mod async_worker;
mod builder;
mod command;
mod errors;
mod status;
mod sync_worker;
mod types;

// Re-export commonly used types for listener pattern
pub use adapters::TokioWatchInput;
pub use adapters::*;
pub use builder::ServiceBuilder;
pub use command::{CommandCompletionSender, CommandHandle};
pub use errors::ServiceError;
pub use status::{AnyStatus, GenericStatusMonitor, ServiceMonitor, StatusMonitor};
pub use types::*;
