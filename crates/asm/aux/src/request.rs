use bitcoin::Txid;
use strata_asm_common::AuxRequestPayload;

/// Supported auxiliary queries that subprotocols can register during preprocessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuxRequestSpec {
    /// Request logs emitted across a contiguous range of L1 blocks (inclusive).
    HistoricalLogs { start_block: u64, end_block: u64 },
    /// Request the full deposit request transaction referenced by the deposit.
    DepositRequestTx { txid: Txid },
}

impl AuxRequestSpec {
    /// Creates a boxed trait object suitable for passing into
    /// [`AuxInputCollector::request_aux_input`].
    pub fn boxed(self) -> Box<dyn AuxRequestPayload> {
        Box::new(self)
    }

    pub fn historical_logs(block: u64) -> Self {
        Self::HistoricalLogs {
            start_block: block,
            end_block: block,
        }
    }

    pub fn historical_logs_range(start_block: u64, end_block: u64) -> Self {
        Self::HistoricalLogs {
            start_block,
            end_block,
        }
    }

    pub fn deposit_request_tx(txid: Txid) -> Self {
        Self::DepositRequestTx { txid }
    }
}
