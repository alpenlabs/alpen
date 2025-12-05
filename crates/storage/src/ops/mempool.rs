//! Mempool data operation interface.

use strata_db_types::{traits::*, types::MempoolTxData};
use strata_identifiers::OLTxId;

use crate::exec::*;

inst_ops_simple! {
    (<D: MempoolDatabase> => MempoolDataOps) {
        put_tx(data: MempoolTxData) => ();
        get_tx(txid: OLTxId) => Option<MempoolTxData>;
        get_all_txs() => Vec<MempoolTxData>;
        del_tx(txid: OLTxId) => bool;
    }
}
