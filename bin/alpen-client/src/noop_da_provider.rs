use alpen_ee_common::{BatchDaProvider, BatchId, L1DaBlockRef};
use async_trait::async_trait;
use bitcoin::{absolute::Height, hashes::Hash, Txid, Wtxid};
use strata_identifiers::L1BlockCommitment;
use strata_primitives::{Buf32, L1BlockId};

/// Simple implementation of [`BatchDaProvider`] that accepts everything as Ok to allow batch
/// lifecycle to proceed. To be replaced once EE DA implementation is completed.
#[expect(unused, reason = "wip")]
pub(crate) struct NoopDaProvider;

#[async_trait]
impl BatchDaProvider for NoopDaProvider {
    async fn post_batch_da(&self, _batch_id: BatchId) -> eyre::Result<Vec<(Txid, Wtxid)>> {
        let txid = Txid::from_raw_hash(Hash::all_zeros());
        let wtxid = Wtxid::from_raw_hash(Hash::all_zeros());

        Ok(vec![(txid, wtxid)])
    }

    async fn check_da_status(
        &self,
        txns: &[(Txid, Wtxid)],
    ) -> eyre::Result<Option<Vec<L1DaBlockRef>>> {
        let block = L1BlockCommitment::new(Height::ZERO, L1BlockId::from(Buf32::zero()));
        let blockrefs = vec![L1DaBlockRef::new(block, txns.to_vec())];

        Ok(Some(blockrefs))
    }
}
