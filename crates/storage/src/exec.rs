//! DB operation interface logic, primarily for generating database operation traits and shim
//! functions.
//!
//! This module provides macros to simplify the creation of both asynchronous and synchronous
//! interfaces for database operations. The macros manage the indirection required to spawn async
//! requests onto a thread pool and execute blocking calls locally.

pub(crate) use strata_db::errors::DbError;
pub(crate) use strata_storage_common::{inst_ops_ctx_shim_generic, inst_ops_generic};

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
