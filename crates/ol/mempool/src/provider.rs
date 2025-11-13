//! Transaction provider trait for RPC-agnostic transaction ingestion.

use futures::Stream;

/// Error type for transaction providers.
#[derive(Debug, thiserror::Error)]
pub enum OLTxProviderError {
    /// Failed to read from the transaction source.
    #[error("Failed to read transaction: {0}")]
    ReadError(String),

    /// The transaction source has been closed or disconnected.
    #[error("Transaction source closed")]
    SourceClosed,

    /// Internal error in the transaction provider.
    #[error("Provider error: {0}")]
    Internal(String),
}

/// Trait for transaction sources that can provide raw transaction blobs.
///
/// This trait allows the mempool to accept transactions from any source:
/// - RPC endpoints
/// - P2P gossip networks
/// - ZMQ queues
/// - Other transports
///
/// The mempool is RPC-agnostic and works with any type that implements this trait.
///
/// # Example
///
/// ```rust,no_run
/// use std::{
///     pin::Pin,
///     task::{Context, Poll},
/// };
///
/// use futures::Stream;
/// use strata_ol_mempool::provider::{OLTxProvider, OLTxProviderError};
///
/// // RPC-based provider
/// struct RpcTxProvider {
///     // ... RPC client ...
/// }
///
/// impl Stream for RpcTxProvider {
///     type Item = Result<Vec<u8>, OLTxProviderError>;
///
///     fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
///         // ... implementation ...
///         Poll::Pending
///     }
/// }
///
/// // RpcTxProvider automatically implements OLTxProvider via the blanket impl
/// // since it implements Stream<Item = Result<Vec<u8>, OLTxProviderError>> + Send + Sync + 'static
/// ```
pub trait OLTxProvider:
    Stream<Item = Result<Vec<u8>, OLTxProviderError>> + Send + Sync + 'static
{
}

/// Blanket implementation: any Stream that yields transaction blobs is an OLTxProvider.
impl<T> OLTxProvider for T where
    T: Stream<Item = Result<Vec<u8>, OLTxProviderError>> + Send + Sync + 'static
{
}
