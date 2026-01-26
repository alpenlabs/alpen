use alpen_ee_common::{BatchDaProvider, BatchId, DaStatus, L1DaBlockRef};
use async_trait::async_trait;
use bitcoin::{hashes::Hash, Txid, Wtxid};
use strata_identifiers::L1BlockCommitment;
use strata_primitives::{Buf32, L1BlockId};

/// Simple implementation of [`BatchDaProvider`] that accepts everything as Ok to allow batch
/// lifecycle to proceed. To be replaced once EE DA implementation is completed.
pub(crate) struct NoopDaProvider;

#[async_trait]
impl BatchDaProvider for NoopDaProvider {
    async fn post_batch_da(&self, _batch_id: BatchId) -> eyre::Result<()> {
        Ok(())
    }

    async fn check_da_status(&self, _batch_id: BatchId) -> eyre::Result<DaStatus> {
        let txid = Txid::from_raw_hash(Hash::all_zeros());
        let wtxid = Wtxid::from_raw_hash(Hash::all_zeros());
        let block = L1BlockCommitment::new(0, L1BlockId::from(Buf32::zero()));

        let blockrefs = vec![L1DaBlockRef::new(block, vec![(txid, wtxid)])];
        Ok(DaStatus::Ready(blockrefs))
    }
}
