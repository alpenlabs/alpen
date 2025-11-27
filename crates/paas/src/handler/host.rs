//! Host resolution abstractions for zkVM backends
//!
//! This module provides the core abstractions for resolving zkVM hosts across
//! different backends (SP1, Risc0, Native, etc.). It defines:
//!
//! - `HostInstance`: Type-erased wrapper for concrete host types
//! - `HostResolver`: Trait for centralizing host resolution logic
//!
//! ## Design
//!
//! Since zkaleido's `ZkVmHost` and `ZkVmRemoteHost` traits are not object-safe
//! (they require `Self: Sized`), we cannot use trait objects like `Arc<dyn ZkVmRemoteHost>`.
//! Instead, `HostInstance` provides an enum-based wrapper that:
//! - Wraps concrete host types in Arc
//! - Provides ergonomic methods that dispatch to the appropriate implementation
//! - Maintains type safety while enabling dynamic dispatch
//!
//! The `HostResolver` trait provides a single entry point for all host resolution,
//! centralizing feature flag checks, backend selection, and host instantiation.

use std::sync::Arc;

use zkaleido::{
    ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram, ZkVmRemoteHost, ZkVmRemoteProgram,
};

use crate::{program::ProgramType, ZkVmBackend};

// ============================================================================
// Host Instance Wrapper
// ============================================================================

/// Wrapper enum for zkVM hosts that provides a unified interface
///
/// Since `ZkVmHost` and `ZkVmRemoteHost` traits are not object-safe (require `Self: Sized`),
/// we cannot use `Arc<dyn ZkVmRemoteHost>`. This enum wraps concrete host types and
/// provides ergonomic methods that dispatch to the appropriate implementation.
///
/// # Type Parameters
///
/// - `R`: Remote host type (implements `ZkVmHost + ZkVmRemoteHost`) - used for SP1, Risc0, etc.
/// - `N`: Native host type (implements `ZkVmHost`) - used for native execution
pub enum HostInstance<R, N>
where
    R: ZkVmHost + Send + Sync,
    N: ZkVmHost + Send + Sync,
{
    /// Remote backend host (supports async remote proving: SP1, Risc0, etc.)
    Remote(Arc<R>),
    /// Native backend host (synchronous local proving)
    Native(Arc<N>),
}

impl<R, N> std::fmt::Debug for HostInstance<R, N>
where
    R: ZkVmHost + Send + Sync,
    N: ZkVmHost + Send + Sync,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Remote(_) => f.debug_tuple("Remote").field(&"<host>").finish(),
            Self::Native(_) => f.debug_tuple("Native").field(&"<host>").finish(),
        }
    }
}

impl<R, N> HostInstance<R, N>
where
    R: ZkVmHost + ZkVmRemoteHost + Send + Sync,
    N: ZkVmHost + Send + Sync,
{
    /// Execute synchronous proving (works for both Remote and Native)
    ///
    /// This calls `ZkVmProgram::prove()` on the underlying host.
    pub fn prove<Prog>(
        &self,
        input: &Prog::Input,
    ) -> Result<ProofReceiptWithMetadata, Box<dyn std::error::Error + Send + Sync>>
    where
        Prog: ZkVmProgram,
    {
        match self {
            Self::Remote(host) => Prog::prove(input, host.as_ref()).map_err(|e| e.into()),
            Self::Native(host) => Prog::prove(input, host.as_ref()).map_err(|e| e.into()),
        }
    }

    /// Start asynchronous remote proving (used by remote backends: SP1, Risc0, etc.)
    ///
    /// Returns a proof ID that can be used to poll for completion.
    /// Only works with Remote hosts; panics if called on Native host.
    pub async fn start_proving<Prog>(
        &self,
        input: &Prog::Input,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>
    where
        Prog: ZkVmRemoteProgram,
    {
        match self {
            Self::Remote(host) => Prog::start_proving(input, host.as_ref())
                .await
                .map_err(|e| e.into()),
            Self::Native(_) => {
                panic!("start_proving called on Native host (should use prove instead)")
            }
        }
    }

    /// Poll for proof completion (used by remote backends: SP1, Risc0, etc.)
    ///
    /// Returns `Some(proof)` if ready, `None` if still computing.
    /// Only works with Remote hosts; panics if called on Native host.
    pub async fn get_proof_if_ready(
        &self,
        proof_id: String,
    ) -> Result<Option<ProofReceiptWithMetadata>, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            Self::Remote(host) => host
                .get_proof_if_ready(proof_id)
                .await
                .map_err(|e| e.into()),
            Self::Native(_) => {
                panic!("get_proof_if_ready called on Native host (not applicable)")
            }
        }
    }
}

impl<R, N> Clone for HostInstance<R, N>
where
    R: ZkVmHost + Send + Sync,
    N: ZkVmHost + Send + Sync,
{
    fn clone(&self) -> Self {
        match self {
            Self::Remote(host) => Self::Remote(Arc::clone(host)),
            Self::Native(host) => Self::Native(Arc::clone(host)),
        }
    }
}

// ============================================================================
// Host Resolution Trait
// ============================================================================

/// Trait for resolving zkVM hosts based on program and backend
///
/// Implement this trait to provide host resolution logic. This is where you:
/// - Handle feature flag checks (e.g., `#[cfg(feature = "sp1")]`)
/// - Instantiate appropriate hosts based on `ProofContext`
/// - Centralize all host creation logic
///
/// # Example
///
/// ```ignore
/// pub struct MyHostResolver;
///
/// impl HostResolver<MyProgramType> for MyHostResolver {
///     type RemoteHost = zkaleido_sp1_host::SP1Host;  // Or Risc0Host, etc.
///     type NativeHost = zkaleido_native_adapter::NativeHost;
///
///     fn resolve(&self, program: &MyProgramType, backend: &ZkVmBackend)
///         -> HostInstance<Self::RemoteHost, Self::NativeHost>
///     {
///         match backend {
///             ZkVmBackend::SP1 => {
///                 #[cfg(feature = "sp1")]
///                 { HostInstance::Remote(create_sp1_host(program)) }
///                 #[cfg(not(feature = "sp1"))]
///                 { panic!("SP1 not enabled") }
///             }
///             ZkVmBackend::Risc0 => {
///                 #[cfg(feature = "risc0")]
///                 { HostInstance::Remote(create_risc0_host(program)) }
///                 #[cfg(not(feature = "risc0"))]
///                 { panic!("Risc0 not enabled") }
///             }
///             ZkVmBackend::Native => HostInstance::Native(create_native_host(program)),
///         }
///     }
/// }
/// ```
pub trait HostResolver<P: ProgramType>: Send + Sync {
    /// The concrete remote host type (SP1, Risc0, or other remote proving backends)
    type RemoteHost: ZkVmHost + ZkVmRemoteHost + Send + Sync;

    /// The concrete native host type (local synchronous proving)
    type NativeHost: ZkVmHost + Send + Sync;

    /// Resolve the appropriate host for the given program and backend
    ///
    /// This is the single entry point for host resolution. All backend selection,
    /// feature flag checks, and host instantiation should happen here.
    fn resolve(
        &self,
        program: &P,
        backend: &ZkVmBackend,
    ) -> HostInstance<Self::RemoteHost, Self::NativeHost>;
}
