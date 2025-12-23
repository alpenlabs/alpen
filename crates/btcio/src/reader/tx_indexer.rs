use strata_asm_types::{
    DepositInfo, DepositSpendInfo, ProtocolOperation, WithdrawalFulfillmentInfo,
};
use strata_checkpoint_types::SignedCheckpoint;
use strata_l1tx::{
    filter::indexer::TxVisitor,
    messages::{DaEntry, L1TxMessages},
};

/// Ops indexer for rollup client. Collects extra info like da blobs and deposit requests
#[derive(Clone, Debug)]
pub(crate) struct ReaderTxVisitorImpl {
    ops: Vec<ProtocolOperation>,
    da_entries: Vec<DaEntry>,
}

impl ReaderTxVisitorImpl {
    pub(crate) fn new() -> Self {
        Self {
            ops: Vec::new(),
            da_entries: Vec::new(),
        }
    }
}

impl TxVisitor for ReaderTxVisitorImpl {
    type Output = L1TxMessages;

    fn visit_da<'a>(&mut self, chunks: impl Iterator<Item = &'a [u8]>) {
        let da_entry = DaEntry::from_chunks(chunks);
        self.ops
            .push(ProtocolOperation::DaCommitment(*da_entry.commitment()));
        self.da_entries.push(da_entry);
    }

    fn visit_deposit(&mut self, d: DepositInfo) {
        self.ops.push(ProtocolOperation::Deposit(d));
    }

    fn visit_checkpoint(&mut self, chkpt: SignedCheckpoint) {
        self.ops.push(ProtocolOperation::Checkpoint(chkpt));
    }

    fn visit_withdrawal_fulfillment(&mut self, info: WithdrawalFulfillmentInfo) {
        self.ops
            .push(ProtocolOperation::WithdrawalFulfillment(info));
    }

    fn visit_deposit_spend(&mut self, info: DepositSpendInfo) {
        self.ops.push(ProtocolOperation::DepositSpent(info));
    }

    fn finalize(self) -> Option<L1TxMessages> {
        if self.ops.is_empty() && self.da_entries.is_empty() {
            None
        } else {
            Some(L1TxMessages::new(self.ops, self.da_entries))
        }
    }
}
