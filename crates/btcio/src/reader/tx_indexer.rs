use strata_l1tx::{
    filter::indexer::TxVisitor,
    messages::{DaEntry, L1TxMessages},
};
use strata_primitives::{
    batch::SignedCheckpoint,
    l1::{
        DepositInfo, DepositRequestInfo, DepositSpendInfo, ProtocolOperation,
        WithdrawalFulfillmentInfo,
    },
};

/// Ops indexer for rollup client. Collects extra info like da blobs and deposit requests
#[derive(Clone, Debug)]
pub(crate) struct ReaderTxVisitorImpl {
    ops: Vec<ProtocolOperation>,
    deposit_requests: Vec<DepositRequestInfo>,
    da_entries: Vec<DaEntry>,
}

impl ReaderTxVisitorImpl {
    pub(crate) fn new() -> Self {
        Self {
            ops: Vec::new(),
            deposit_requests: Vec::new(),
            da_entries: Vec::new(),
        }
    }

    pub(crate) fn ops(&self) -> &[ProtocolOperation] {
        &self.ops
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

    fn visit_deposit_request(&mut self, dr: DepositRequestInfo) {
        self.deposit_requests.push(dr);
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
        if self.ops.is_empty() && self.deposit_requests.is_empty() && self.da_entries.is_empty() {
            None
        } else {
            Some(L1TxMessages::new(
                self.ops,
                self.deposit_requests,
                self.da_entries,
            ))
        }
    }
}

#[cfg(test)]
mod test {
    use strata_test_utils::tx_indexer::{
        test_index_deposit_request_with_visitor, test_index_deposit_with_visitor,
        test_index_multiple_deposits_with_visitor, test_index_no_deposit_with_visitor,
        test_index_tx_with_multiple_ops_with_visitor,
        test_index_withdrawal_fulfillment_with_visitor,
    };

    use crate::reader::tx_indexer::ReaderTxVisitorImpl;

    #[test]
    fn test_index_deposits() {
        let _ = test_index_deposit_with_visitor(ReaderTxVisitorImpl::new, |tx| {
            tx.item().protocol_ops().to_vec()
        });
    }

    #[ignore = "Ignored because deposit request is not included as ops"]
    #[test]
    fn test_index_txs_deposit_request() {
        let _ = test_index_deposit_request_with_visitor(ReaderTxVisitorImpl::new, |ind_output| {
            ind_output.item().protocol_ops().to_vec()
        });
    }

    #[test]
    fn test_index_no_deposit() {
        let _ = test_index_no_deposit_with_visitor(ReaderTxVisitorImpl::new, |ind_output| {
            ind_output.item().protocol_ops().to_vec()
        });
    }

    #[test]
    fn test_index_multiple_deposits() {
        let _ = test_index_multiple_deposits_with_visitor(ReaderTxVisitorImpl::new, |ind_output| {
            ind_output.item().protocol_ops().to_vec()
        });
    }

    #[test]
    fn test_index_tx_with_multiple_ops() {
        let _ =
            test_index_tx_with_multiple_ops_with_visitor(ReaderTxVisitorImpl::new, |ind_output| {
                ind_output.item().protocol_ops().to_vec()
            });
    }

    #[test]
    fn test_index_withdrawal_fulfillment() {
        let _ = test_index_withdrawal_fulfillment_with_visitor(
            ReaderTxVisitorImpl::new,
            |ind_output| ind_output.item().protocol_ops().to_vec(),
        );
    }
}
