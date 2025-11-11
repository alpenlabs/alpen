use strata_db_types::{traits::*, types::L1TxEntry};
use strata_primitives::buf::Buf32;

use crate::exec::*;

inst_ops_simple! {
    (<D: L1BroadcastDatabase> => BroadcastDbOps) {
        get_tx_entry(idx: u64) => Option<L1TxEntry>;
        get_tx_entry_by_id(id: Buf32) => Option<L1TxEntry>;
        get_txid(idx: u64) => Option<Buf32>;
        get_next_tx_idx() => u64;
        put_tx_entry(id: Buf32, entry: L1TxEntry) => Option<u64>;
        put_tx_entry_by_idx(idx: u64, entry: L1TxEntry) => ();
        get_last_tx_entry() => Option<L1TxEntry>;
    }
}
