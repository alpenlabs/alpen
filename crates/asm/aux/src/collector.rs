use std::{any::Any, collections::BTreeMap};

use strata_asm_common::{AuxRequestCollector, AuxRequestSpec, L1TxIndex};

use crate::request::{self, AuxRequestEnvelope};

/// Collector that records auxiliary requests keyed by L1 transaction index.
#[derive(Debug, Default)]
pub struct RequestCollector {
    requests: BTreeMap<L1TxIndex, AuxRequestSpec>,
}

impl RequestCollector {
    pub fn new() -> Self {
        Self {
            requests: BTreeMap::new(),
        }
    }

    pub fn into_requests(self) -> AuxRequestEnvelope {
        let mut asm_log_queries = Vec::new();
        let mut drt_tx_queries = Vec::new();

        for (tx_index, request) in self.requests {
            match request {
                AuxRequestSpec::AsmLogQueries {
                    start_block,
                    end_block,
                } => {
                    asm_log_queries.push(request::AuxLogQuery {
                        requester_tx_index: tx_index,
                        start_block,
                        end_block,
                    });
                }
                AuxRequestSpec::DepositRequestTx { txid } => {
                    drt_tx_queries.push(request::DrtTxQuery {
                        requester_tx_index: tx_index,
                        drt_txid: txid,
                    });
                }
            }
        }

        AuxRequestEnvelope {
            asm_log_queries,
            drt_tx_queries,
        }
    }
}

impl AuxRequestCollector for RequestCollector {
    fn request_aux_input(&mut self, tx_index: L1TxIndex, request: AuxRequestSpec) {
        // guard against duplicate requests for the same tx_index
        if self.requests.contains_key(&tx_index) {
            panic!("Auxiliary request for tx_index {} already exists", tx_index);
        }
        self.requests.insert(tx_index, request);
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}
