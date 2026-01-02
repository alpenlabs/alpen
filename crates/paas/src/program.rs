//! Program type abstraction for dynamic dispatch
//!
//! This module defines the core `ProgramType` trait that enables PaaS to work
//! with arbitrary program types while maintaining type safety and routing capabilities.
//!
//! The trait allows extracting a routing key from any program type, which PaaS
//! uses to dispatch tasks to the appropriate handler.

use std::{fmt, hash::Hash};

use serde::{Deserialize, Serialize};

/// Trait that program types must implement for dynamic dispatch
///
/// This trait allows PaaS to extract a routing key from any program type
/// for handler routing, while maintaining full type information throughout
/// the proof generation pipeline.
///
/// # Type Parameters
///
/// - `RoutingKey`: The key type used for handler lookup (typically an enum variant discriminant)
///
/// # Example
///
/// ```ignore
/// #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// pub enum MyProgram {
///     ProgramA(DataA),
///     ProgramB(DataB),
/// }
///
/// #[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// pub enum ProgramVariant {
///     A,
///     B,
/// }
///
/// impl ProgramType for MyProgram {
///     type RoutingKey = ProgramVariant;
///
///     fn routing_key(&self) -> Self::RoutingKey {
///         match self {
///             MyProgram::ProgramA(_) => ProgramVariant::A,
///             MyProgram::ProgramB(_) => ProgramVariant::B,
///         }
///     }
/// }
/// ```
pub trait ProgramType:
    Clone + Eq + Hash + Send + Sync + fmt::Debug + Serialize + for<'de> Deserialize<'de> + 'static
{
    /// Routing key type (typically an enum discriminant)
    ///
    /// This is used by PaaS to look up the correct handler for a given program.
    type RoutingKey: Eq + Hash + Clone + Send + Sync + fmt::Debug + 'static;

    /// Extract the routing key for handler lookup
    ///
    /// This method should return a stable key that uniquely identifies which
    /// handler should process this program type.
    fn routing_key(&self) -> Self::RoutingKey;
}
