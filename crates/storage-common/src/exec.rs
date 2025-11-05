//! DB operation interface logic, providing generic macros for generating database operation
//! traits and shim functions.
//!
//! This module provides the core `inst_ops!` and `inst_ops_generic!` macros that simplify the
//! creation of both asynchronous and synchronous interfaces for database operations. The macros
//! manage the indirection required to spawn async requests onto a thread pool and execute
//! blocking calls locally.
//!
//! ## Architecture
//!
//! - `inst_ops!` - Low-level macro that accepts a custom error type parameter
//! - `inst_ops_generic!` - High-level macro that generates a complete ops interface with custom
//!   error types
//! - `inst_ops_ctx_shim_generic!` - Helper macro for generating context shim functions
//!
//! These macros are designed to be error-type agnostic, allowing different crates to use
//! their own error types while maintaining the same convenient interface generation.

use thiserror::Error;

/// Handle for receiving a result from a database operation with a generic error type.
pub type GenericRecv<T, E> = tokio::sync::oneshot::Receiver<Result<T, E>>;

/// Errors specific to the ops execution layer.
///
/// These errors represent failures in the operation execution infrastructure itself,
/// not database-level errors. The most common case is when a worker thread fails to
/// send a response back through the channel.
#[derive(Debug, Clone, Error)]
pub enum OpsError {
    #[error("worker failed strangely")]
    WorkerFailedStrangely,
}

/// Macro to generate an `Ops` interface, which provides both asynchronous and synchronous
/// methods for interacting with the underlying database. This is particularly useful for
/// defining database operations in a consistent and reusable manner.
///
/// This is a low-level macro used internally by `inst_ops_generic!`. Most users should use
/// `inst_ops_generic!` instead.
///
/// ### Usage
///
/// The macro defines an operations trait for a specified context, error type, and a list of
/// methods. Each method in the generated interface will have both `async` and `sync` variants.
///
/// ```ignore
/// inst_ops! {
///     (InscriptionDataOps, Context<D: SequencerDatabase>, MyError) {
///         get_blob_entry(id: Buf32) => Option<PayloadEntry>;
///         get_blob_entry_by_idx(idx: u64) => Option<PayloadEntry>;
///         get_blob_entry_id(idx: u64) => Option<Buf32>;
///         get_next_blob_idx() => u64;
///         put_blob_entry(id: Buf32, entry: PayloadEntry) => ();
///     }
/// }
/// ```
///
/// Requires corresponding function definitions:
///
/// ```ignore
/// fn get_blob_entry<D: Database>(ctx: &Context<D>, id: Buf32) -> Result<Option<PayloadEntry>, MyError> { ... }
///
/// fn put_blob_entry<D: Database>(ctx: &Context<D>, id: Buf32, entry: PayloadEntry) -> Result<(), MyError> { ... }
///
/// // ... Other definitions corresponding to above macro invocation
/// ```
///
/// - **`InscriptionDataOps`**: The name of the operations interface being generated.
/// - **`Context<D: SequencerDatabase>`**: The context type that the operations will act upon. This
///   usually wraps the database or related dependencies.
/// - **`MyError`**: The error type for operations. Must implement `From<OpsError>`.
/// - **Method definitions**: Specify the function name, input parameters, and return type. The
///   macro will automatically generate both async and sync variants of these methods.
///
/// This macro simplifies the definition and usage of database operations by reducing boilerplate
/// code and ensuring uniformity in async/sync APIs and by allowing to avoid the generic `<D>`
/// parameter.
#[macro_export]
macro_rules! inst_ops {
    {
        ($base:ident, $ctx:ident $(<$($tparam:ident: $tpconstr:tt),+>)?, $error:ty) {
            $($iname:ident($($aname:ident: $aty:ty),*) => $ret:ty;)*
        }
    } => {
        #[expect(missing_debug_implementations, reason = "Some inner types don't have Debug implementation")]
        pub struct $base {
            pool: $crate::_threadpool::ThreadPool,
            inner: ::std::sync::Arc<dyn ShimTrait>,
        }

        $crate::_paste::paste! {
            impl $base {
                pub fn new $(<$($tparam: $tpconstr + Sync + Send + 'static),+>)? (pool: $crate::_threadpool::ThreadPool, ctx: ::std::sync::Arc<$ctx $(<$($tparam),+>)?>) -> Self {
                    Self {
                        pool,
                        inner: ::std::sync::Arc::new(Inner { ctx }),
                    }
                }

                $(
                    pub async fn [<$iname _async>] (&self, $($aname: $aty),*) -> Result<$ret, $error> {
                        let resp_rx = self.inner. [<$iname _chan>] (&self.pool, $($aname),*);
                        match resp_rx.await {
                            Ok(v) => v,
                            Err(_e) => Err(<$error>::from($crate::exec::OpsError::WorkerFailedStrangely)),
                        }
                    }

                    pub fn [<$iname _blocking>] (&self, $($aname: $aty),*) -> Result<$ret, $error> {
                        self.inner. [<$iname _blocking>] ($($aname),*)
                    }

                    pub fn [<$iname _chan>] (&self, $($aname: $aty),*) -> $crate::exec::GenericRecv<$ret, $error> {
                        self.inner. [<$iname _chan>] (&self.pool, $($aname),*)
                    }
                )*
            }

            #[async_trait::async_trait]
            trait ShimTrait: Sync + Send + 'static {
                $(
                    fn [<$iname _blocking>] (&self, $($aname: $aty),*) -> Result<$ret, $error>;
                    fn [<$iname _chan>] (&self, pool: &$crate::_threadpool::ThreadPool, $($aname: $aty),*) -> $crate::exec::GenericRecv<$ret, $error>;
                )*
            }

            #[derive(Debug)]
            pub struct Inner $(<$($tparam: $tpconstr + Sync + Send + 'static),+>)? {
                ctx: ::std::sync::Arc<$ctx $(<$($tparam),+>)?>,
            }

            impl $(<$($tparam: $tpconstr + Sync + Send + 'static),+>)? ShimTrait for Inner $(<$($tparam),+>)? {
                $(
                    fn [<$iname _blocking>] (&self, $($aname: $aty),*) -> Result<$ret, $error> {
                        $iname(&self.ctx, $($aname),*)
                    }

                    fn [<$iname _chan>] (&self, pool: &$crate::_threadpool::ThreadPool, $($aname: $aty),*) -> $crate::exec::GenericRecv<$ret, $error> {
                        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                        let ctx = self.ctx.clone();

                        pool.execute(move || {
                            let res = $iname(&ctx, $($aname),*);
                            if resp_tx.send(res).is_err() {
                                ::tracing::warn!("failed to send response");
                            }
                        });

                        resp_rx
                    }
                )*
            }
        }
    }
}

/// Automatically generates an `Ops` interface with shim functions for database operations within a
/// context without having to define any extra functions, with support for custom error types.
///
/// This macro generates a complete database operations interface including context management,
/// async/sync method variants, and automatic shim function generation. Unlike `inst_ops!`, this
/// macro generates both the context struct and shim functions automatically.
///
/// ### Usage
/// ```ignore
/// inst_ops_generic! {
///     (<D: L1BroadcastDatabase> => BroadcastDbOps, CustomError) {
///         get_tx_entry(idx: u64) => Option<()>;
///         get_tx_entry_by_id(id: u32) => Option<()>;
///         get_txid(idx: u64) => Option<u32>;
///         get_next_tx_idx() => u64;
///         put_tx_entry(id: u32, entry: u64) => Option<u64>;
///         put_tx_entry_by_idx(idx: u64, entry: u32) => ();
///         get_last_tx_entry() => Option<u32>;
///     }
/// }
/// ```
///
/// - **Database trait**: The trait bound for the database type (e.g., `L1BroadcastDatabase`).
/// - **Ops struct name**: The name of the generated operations interface (e.g., `BroadcastDbOps`).
/// - **Error type**: The custom error type to use (e.g., `CustomError`). This error type must
///   implement `From<OpsError>` to handle conversion of internal errors.
/// - **Methods**: Each operation is defined with its inputs and outputs, generating async and sync
///   variants automatically.
///
/// ### Generated Components
/// - `Context<D>` struct wrapping the database
/// - `{OpName}` struct with `new()` and operation methods
/// - Shim functions delegating to database methods
///
/// ### Requirements
/// The custom error type must:
/// - Implement `From<OpsError>` for error conversion
/// - Implement `std::error::Error + Send + Sync + 'static`
///
/// The database must implement all the methods defined in the macro invocation.
#[macro_export]
macro_rules! inst_ops_generic {
    {
        (< $tparam:ident: $tpconstr:tt > => $base:ident, $error:ty) {
            $($iname:ident($($aname:ident: $aty:ty),*) => $ret:ty;)*
        }
    } => {
        #[derive(Debug)]
        pub struct Context<$tparam : $tpconstr> {
            db: ::std::sync::Arc<$tparam>,
        }

        impl<$tparam : $tpconstr + Sync + Send + 'static> Context<$tparam> {
            pub fn new(db: ::std::sync::Arc<$tparam>) -> Self {
                Self { db }
            }

            pub fn into_ops(self, pool: $crate::_threadpool::ThreadPool) -> $base {
                $base::new(pool, ::std::sync::Arc::new(self))
            }
        }

        $crate::inst_ops! {
            ($base, Context<$tparam : $tpconstr>, $error) {
                $($iname ($($aname : $aty ),*) => $ret ;)*
            }
        }

        $(
            inst_ops_ctx_shim_generic!($iname<$tparam: $tpconstr>($($aname: $aty),*) -> $ret, $error);
        )*
    }
}

/// A macro that generates the context shim functions with a generic error type. This assumes that
/// the `Context` struct has a `db` attribute and that the db object has all the methods defined.
#[macro_export]
macro_rules! inst_ops_ctx_shim_generic {
    ($iname:ident<$tparam: ident : $tpconstr:tt>($($aname:ident: $aty:ty),*) -> $ret:ty, $error:ty) => {
        fn $iname < $tparam : $tpconstr > (context: &Context<$tparam>, $($aname : $aty),* ) -> Result<$ret, $error> {
            context.db.as_ref(). $iname ( $($aname),* )
        }
    }
}
