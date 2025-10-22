use std::{any::Any, collections::BTreeMap};

use strata_asm_common::{AuxInputCollector, AuxRequest, AuxRequestPayload, L1TxIndex};

/// Collector that records auxiliary requests keyed by L1 transaction index.
#[derive(Default)]
pub struct AuxRequestCollector {
    requests: BTreeMap<L1TxIndex, Box<dyn AuxRequestPayload>>,
}

impl AuxRequestCollector {
    pub fn new() -> Self {
        Self {
            requests: BTreeMap::new(),
        }
    }

    pub fn into_requests(self) -> Vec<AuxRequest> {
        self.requests
            .into_iter()
            .map(|(tx_index, payload)| AuxRequest::new(tx_index, payload))
            .collect()
    }
}

impl std::fmt::Debug for AuxRequestCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuxRequestCollector")
            .field("pending", &self.requests.len())
            .finish()
    }
}

impl AuxInputCollector for AuxRequestCollector {
    fn request_aux_input(&mut self, tx_index: L1TxIndex, payload: Box<dyn AuxRequestPayload>) {
        if self.requests.insert(tx_index, payload).is_some() {
            panic!("asm: duplicate aux request for L1 tx index {tx_index}");
        }
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}
