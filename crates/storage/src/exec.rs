//! DB operation interface logic, primarily for generating database operation traits and shim
//! functions.
//!
//! This module provides macros to simplify the creation of both asynchronous and synchronous
//! interfaces for database operations. The macros manage the indirection required to spawn async
//! requests onto a thread pool and execute blocking calls locally.

pub(crate) use strata_db::{errors::DbError, DbResult};
pub(crate) use tracing::*;

/// Handle for receiving a result from a database operation on another thread.
pub(crate) type DbRecv<T> = tokio::sync::oneshot::Receiver<DbResult<T>>;

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
///     (InscriptionDataOps, Context<D: SequencerDatabase>) {
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
macro_rules! inst_ops {
    {
        ($base:ident, $ctx:ident $(<$($tparam:ident: $tpconstr:tt),+>)?) {
            $($iname:ident($($aname:ident: $aty:ty),*) => $ret:ty;)*
        }
    } => {
        #[expect(missing_debug_implementations)]
        pub struct $base {
            pool: threadpool::ThreadPool,
            inner: Arc<dyn ShimTrait>,
        }

        paste::paste! {
            impl $base {
                pub fn new $(<$($tparam: $tpconstr + Sync + Send + 'static),+>)? (pool: threadpool::ThreadPool, ctx: Arc<$ctx $(<$($tparam),+>)?>) -> Self {
                    Self {
                        pool,
                        inner: Arc::new(Inner { ctx }),
                    }
                }

                $(
                    pub async fn [<$iname _async>] (&self, $($aname: $aty),*) -> DbResult<$ret> {
                        let resp_rx = self.inner. [<$iname _chan>] (&self.pool, $($aname),*);
                        match resp_rx.await {
                            Ok(v) => v,
                            Err(_e) => Err(DbError::WorkerFailedStrangely),
                        }
                    }

                    pub fn [<$iname _blocking>] (&self, $($aname: $aty),*) -> DbResult<$ret> {
                        self.inner. [<$iname _blocking>] ($($aname),*)
                    }

                    pub fn [<$iname _chan>] (&self, $($aname: $aty),*) -> DbRecv<$ret> {
                        self.inner. [<$iname _chan>] (&self.pool, $($aname),*)
                    }
                )*
            }

            #[async_trait::async_trait]
            trait ShimTrait: Sync + Send + 'static {
                $(
                    fn [<$iname _blocking>] (&self, $($aname: $aty),*) -> DbResult<$ret>;
                    fn [<$iname _chan>] (&self, pool: &threadpool::ThreadPool, $($aname: $aty),*) -> DbRecv<$ret>;
                )*
            }

            #[derive(Debug)]
            pub struct Inner $(<$($tparam: $tpconstr + Sync + Send + 'static),+>)? {
                ctx: Arc<$ctx $(<$($tparam),+>)?>,
            }

            impl $(<$($tparam: $tpconstr + Sync + Send + 'static),+>)? ShimTrait for Inner $(<$($tparam),+>)? {
                $(
                    fn [<$iname _blocking>] (&self, $($aname: $aty),*) -> DbResult<$ret> {
                        $iname(&self.ctx, $($aname),*)
                    }

                    fn [<$iname _chan>] (&self, pool: &threadpool::ThreadPool, $($aname: $aty),*) -> DbRecv<$ret> {
                        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                        let ctx = self.ctx.clone();

                        pool.execute(move || {
                            let res = $iname(&ctx, $($aname),*);
                            if resp_tx.send(res).is_err() {
                                warn!("failed to send response");
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
/// context without having to define any extra functions.
///
/// ### Usage
/// ```ignore
/// inst_ops_simple! {
///     (<D: L1BroadcastDatabase> => BroadcastDbOps) {
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
/// - **Methods**: Each operation is defined with its inputs and outputs, generating async and sync
///   variants automatically.
macro_rules! inst_ops_simple {
    {
        (< $tparam:ident: $tpconstr:tt > => $base:ident) {
            $($iname:ident($($aname:ident: $aty:ty),*) => $ret:ty;)*
        }
    } => {
        #[derive(Debug)]
        pub struct Context<$tparam : $tpconstr> {
            db: Arc<$tparam>,
        }

        impl<$tparam : $tpconstr + Sync + Send + 'static> Context<$tparam> {
            pub const fn new(db: Arc<$tparam>) -> Self {
                Self { db }
            }

            pub fn into_ops(self, pool: threadpool::ThreadPool) -> $base {
                $base::new(pool, Arc::new(self))
            }
        }

        inst_ops! {
            ($base, Context<$tparam : $tpconstr>) {
                $($iname ($($aname : $aty ),*) => $ret ;)*
            }
        }

        $(
            inst_ops_ctx_shim!($iname<$tparam: $tpconstr>($($aname: $aty),*) -> $ret);
        )*
    }
}

/// A macro that generates the context shim functions. This assumes that the `Context`
/// struct has a `db` attribute and that the db object has all the methods defined.
macro_rules! inst_ops_ctx_shim {
    ($iname:ident<$tparam: ident : $tpconstr:tt>($($aname:ident: $aty:ty),*) -> $ret:ty) => {
        fn $iname < $tparam : $tpconstr > (context: &Context<$tparam>, $($aname : $aty),* ) -> DbResult<$ret> {
            context.db.as_ref(). $iname ( $($aname),* )
        }
    }
}

pub(crate) use inst_ops;
pub(crate) use inst_ops_ctx_shim;
pub(crate) use inst_ops_simple;
