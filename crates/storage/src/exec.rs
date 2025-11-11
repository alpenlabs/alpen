//! DB operation interface logic for strata-storage crate.
//!
//! This module provides the `inst_ops_simple!` macro, which is a specialized version of
//! `inst_ops_generic!` (from `strata-storage-common`) that defaults to using `DbError`.
//!
//! For custom error types, use `inst_ops_generic!` directly from `strata-storage-common`.

pub(crate) use strata_db_types::errors::DbError;
pub(crate) use strata_storage_common::{inst_ops_ctx_shim_generic, inst_ops_generic};

/// Automatically generates an `Ops` interface with shim functions for database operations within a
/// context without having to define any extra functions, using `DbError` as the error type.
///
/// This is a convenience wrapper around `inst_ops_generic!` that defaults to using `DbError`.
/// If you need a custom error type, use `inst_ops_generic!` from `strata-storage-common` instead.
///
/// The macro generates:
/// - A `Context<T>` struct that wraps the database
/// - An ops struct with async, blocking, and channel-based methods
/// - Shim functions that delegate to the database methods
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
/// ### Parameters
/// - **Database trait**: The trait bound for the database (e.g., `L1BroadcastDatabase`)
/// - **Ops struct name**: The name of the generated operations struct (e.g., `BroadcastDbOps`)
/// - **Methods**: Each operation is defined with its inputs and outputs
///
/// ### Generated API
/// For each method `foo(arg: Type) => ReturnType`, the macro generates:
/// - `foo_async(&self, arg: Type) -> DbResult<ReturnType>` - Async version
/// - `foo_blocking(&self, arg: Type) -> DbResult<ReturnType>` - Blocking version
/// - `foo_chan(&self, arg: Type) -> DbRecv<ReturnType>` - Channel-based version
///
/// ### Note
/// This macro uses `DbError` from `strata-db`. For custom error types, use `inst_ops_generic!`
/// from the `strata-storage-common` crate.
#[macro_export]
macro_rules! inst_ops_simple {
    (
        ( < $tparam:ident : $tpconstr:tt > => $base:ident )
        {
            $(
                $iname:ident ( $( $aname:ident : $aty:ty ),* $(,)? ) => $ret:ty;
            )* $(,)?
        }
    ) => {
        inst_ops_generic! {
            ( < $tparam : $tpconstr > => $base, DbError )
            {
                $( $iname( $( $aname : $aty ),* ) => $ret; )*
            }
        }
    };
}

pub(crate) use inst_ops_simple;
