use std::collections::BTreeMap;

use bitcoin::Txid;
use strata_asm_common::{L1TxIndex, SubprotocolId};

/// Table mapping subprotocol IDs to their corresponding auxiliary requests.
pub type AuxRequestTable = BTreeMap<SubprotocolId, AuxRequestEnvelope>;
#[derive(Debug)]
pub struct AuxRequestEnvelope {
    pub asm_log_queries: Vec<AuxLogQuery>,
    pub drt_tx_queries: Vec<DrtTxQuery>,
}

impl AuxRequestEnvelope {
    pub fn is_empty(&self) -> bool {
        self.asm_log_queries.is_empty() && self.drt_tx_queries.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuxLogQuery {
    pub requester_tx_index: L1TxIndex,
    pub start_block: u64,
    pub end_block: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrtTxQuery {
    pub requester_tx_index: L1TxIndex,
    pub drt_txid: Txid,
}
