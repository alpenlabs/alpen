use strata_l1tx::filter::indexer::TxVisitor;
use strata_primitives::{
    batch::SignedCheckpoint,
    l1::{
        DaCommitment, DepositInfo, DepositSpendInfo, ProtocolOperation, WithdrawalFulfillmentInfo,
    },
};

/// Ops indexer for use with the prover.
///
/// This just extracts *only* the protocol operations, in particular avoiding
/// copying the DA payload again, since memory copies are more expensive in
/// proofs.
#[derive(Debug, Clone)]
pub(crate) struct ProverTxVisitorImpl {
    ops: Vec<ProtocolOperation>,
}

impl ProverTxVisitorImpl {
    pub(crate) fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl TxVisitor for ProverTxVisitorImpl {
    type Output = Vec<ProtocolOperation>;

    fn visit_da<'a>(&mut self, chunks: impl Iterator<Item = &'a [u8]>) {
        let commitment = DaCommitment::from_chunk_iter(chunks);
        self.ops.push(ProtocolOperation::DaCommitment(commitment));
    }

    fn visit_deposit(&mut self, di: DepositInfo) {
        self.ops.push(ProtocolOperation::Deposit(di));
    }

    fn visit_checkpoint(&mut self, ckpt: SignedCheckpoint) {
        self.ops.push(ProtocolOperation::Checkpoint(ckpt));
    }

    fn visit_withdrawal_fulfillment(&mut self, info: WithdrawalFulfillmentInfo) {
        self.ops
            .push(ProtocolOperation::WithdrawalFulfillment(info));
    }

    fn visit_deposit_spend(&mut self, info: DepositSpendInfo) {
        self.ops.push(ProtocolOperation::DepositSpent(info));
    }

    fn finalize(self) -> Option<Vec<ProtocolOperation>> {
        if self.ops.is_empty() {
            None
        } else {
            Some(self.ops)
        }
    }
}

/// These are mostly similar to the ones in `strata_btcio::reader::ops_visitor` except for the
/// visitor `ProverOpsVisitor` and indexing of deposit requests.
#[cfg(test)]
mod test {
    use strata_test_utils::tx_indexer::{
        test_index_deposit_request_with_visitor, test_index_deposit_with_visitor,
        test_index_multiple_deposits_with_visitor, test_index_no_deposit_with_visitor,
        test_index_tx_with_multiple_ops_with_visitor,
        test_index_withdrawal_fulfillment_with_visitor,
    };

    use super::ProverTxVisitorImpl;

    #[test]
    fn test_index_deposits() {
        let _ = test_index_deposit_with_visitor(ProverTxVisitorImpl::new, |ind_output| {
            ind_output.item().clone()
        });
    }

    #[ignore = "Ignored because deposit request is not included as ops"]
    #[test]
    fn test_index_txs_deposit_request() {
        let _ = test_index_deposit_request_with_visitor(ProverTxVisitorImpl::new, |ind_output| {
            ind_output.item().clone()
        });
    }

    #[test]
    fn test_index_no_deposit() {
        let _ = test_index_no_deposit_with_visitor(ProverTxVisitorImpl::new, |ind_output| {
            ind_output.item().clone()
        });
    }

    #[test]
    fn test_index_multiple_deposits() {
        let _ = test_index_multiple_deposits_with_visitor(ProverTxVisitorImpl::new, |op_txs| {
            op_txs.item().clone()
        });
    }

    #[test]
    fn test_index_tx_with_multiple_ops() {
        let _ =
            test_index_tx_with_multiple_ops_with_visitor(ProverTxVisitorImpl::new, |ind_output| {
                ind_output.item().clone()
            });
    }

    #[test]
    fn test_index_withdrawal_fulfillment() {
        let _ = test_index_withdrawal_fulfillment_with_visitor(
            ProverTxVisitorImpl::new,
            |ind_output| ind_output.item().clone(),
        );
    }
}
