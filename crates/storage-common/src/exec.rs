//! DB operation interface logic, primarily for generating database operation traits and shim
//! functions.
//!
//! This module provides macros to simplify the creation of both asynchronous and synchronous
//! interfaces for database operations. The macros manage the indirection required to spawn async
//! requests onto a thread pool and execute blocking calls locally.

use thiserror::Error;

/// Handle for receiving a result from a database operation with a generic error type.
pub type GenericRecv<T, E> = tokio::sync::oneshot::Receiver<Result<T, E>>;

/// Errors specific to th
#[derive(Debug, Clone, Error)]
pub enum OpsError {
    #[error("worked failed strangely")]
    WorkerFailedStrangely,
}

/// Macro to generate an `Ops` interface, which provides both asynchronous and synchronous
/// methods for interacting with the underlying database. This is particularly useful for
/// defining database operations in a consistent and reusable manner.
///
/// ### Usage
///
/// The macro defines an operations trait for a specified context and a list of methods.
/// Each method in the generated interface will have both `async` and `sync` variants.
///
/// ```ignore
/// inst_ops! {
///     (InscriptionDataOps, Context<D: SequencerDatabase>, DbError) {
///         get_blob_entry(id: Buf32) => Option<PayloadEntry>;
///         get_blob_entry_by_idx(idx: u64) => Option<PayloadEntry>;
///         get_blob_entry_id(idx: u64) => Option<Buf32>;
///         get_next_blob_idx() => u64;
///         put_blob_entry(id: Buf32, entry: PayloadEntry) => ();
///     }
/// }
/// ```
///
/// Definitions corresponding to above macro invocation:
///
/// ```ignore
/// fn get_blob_entry<D: Database>(ctx: Context<D>, id: u32) -> DbResult<Option<u32>> { ... }
///
/// fn put_blob_entry<D: Database>(ctx: Context<D>, id: Buf32) -> DbResult<()> { ... }
///
/// // ... Other definitions corresponding to above macro invocation
/// ```
///
/// - **`InscriptionDataOps`**: The name of the operations interface being generated.
/// - **`Context<D: SequencerDatabase>`**: The context type that the operations will act upon.This
///   usually wraps the database or related dependencies.
/// - **Method definitions**: Specify the function name, input parameters, and return type.The macro
///   will automatically generate both async and sync variants of these methods.
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
            pool: $crate::threadpool::ThreadPool,
            inner: ::std::sync::Arc<dyn ShimTrait>,
        }

        $crate::paste::paste! {
            impl $base {
                pub fn new $(<$($tparam: $tpconstr + Sync + Send + 'static),+>)? (pool: $crate::threadpool::ThreadPool, ctx: ::std::sync::Arc<$ctx $(<$($tparam),+>)?>) -> Self {
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
                    fn [<$iname _chan>] (&self, pool: &$crate::threadpool::ThreadPool, $($aname: $aty),*) -> $crate::exec::GenericRecv<$ret, $error>;
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

                    fn [<$iname _chan>] (&self, pool: &$crate::threadpool::ThreadPool, $($aname: $aty),*) -> $crate::exec::GenericRecv<$ret, $error> {
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
/// This macro is similar to `inst_ops_simple!` but allows you to specify a custom error type
/// instead of the default `DbError`. This is useful when you want to use domain-specific error
/// types while maintaining the same convenient interface generation.
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
/// - **Context**: Defines the database type (e.g., `L1BroadcastDatabase`).
/// - **Trait**: Maps to the generated interface (e.g., `BroadcastDbOps`).
/// - **Error Type**: The custom error type to use (e.g., `CustomError`). This error type must
///   implement `From<OpsError>` to handle conversion of internal errors.
/// - **Methods**: Each operation is defined with its inputs and outputs, generating async and sync
///   variants automatically.
///
/// ### Requirements
/// The custom error type must:
/// - Implement `From<OpsError>` for error conversion
/// - Implement `std::error::Error + Send + Sync + 'static`
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

            pub fn into_ops(self, pool: $crate::threadpool::ThreadPool) -> $base {
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
