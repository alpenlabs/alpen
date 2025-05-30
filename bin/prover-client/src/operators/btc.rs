use std::sync::Arc;

use bitcoind_async_client::{traits::Reader, Client};
use jsonrpsee::http_client::HttpClient;
use strata_l1tx::filter::types::TxFilterConfig;
use strata_primitives::{
    l1::L1BlockCommitment,
    params::RollupParams,
    proof::{Epoch, ProofContext, ProofKey},
};
use strata_proofimpl_btc_blockspace::{logic::BlockScanProofInput, program::BtcBlockspaceProgram};
use strata_rocksdb::prover::db::ProofDb;
use strata_rpc_api::StrataApiClient;
use strata_state::chain_state::Chainstate;
use tracing::error;

use super::ProvingOp;
use crate::errors::ProvingTaskError;

/// A struct that implements the [`ProvingOp`] trait for Bitcoin blockspace proof generation.
///
/// It interfaces with the Bitcoin blockchain via a [`Client`] to fetch the necessary data
/// required by the [`BtcBlockspaceProgram`] for the proof generation.
#[derive(Debug, Clone)]
pub(crate) struct BtcBlockspaceOperator {
    pub btc_client: Arc<Client>,
    cl_client: HttpClient,
    rollup_params: Arc<RollupParams>,
}

impl BtcBlockspaceOperator {
    /// Creates a new BTC operations instance.
    #[expect(clippy::missing_const_for_fn)]
    pub(crate) fn new(
        btc_client: Arc<Client>,
        cl_client: HttpClient,
        rollup_params: Arc<RollupParams>,
    ) -> Self {
        Self {
            btc_client,
            cl_client,
            rollup_params,
        }
    }

    async fn construct_tx_filter_config(
        &self,
        epoch: u64,
    ) -> Result<TxFilterConfig, ProvingTaskError> {
        let mut tx_filters =
            TxFilterConfig::derive_from(&self.rollup_params).expect("failed to derive tx filters");

        if epoch < 2 {
            return Ok(tx_filters);
        }

        // Chainstate based on which the TxFilterRule needs to be updated
        let epoch_commitments = self
            .cl_client
            .get_epoch_commitments(epoch - 2)
            .await
            .inspect_err(|_| error!(%epoch, "Failed to fetch epoch commitment"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;

        // Sanity check that there is only one epoch commitment for a given epoch
        // TODO: if there are multiple epoch commitments we need a way to handle that to determine
        // canonical commitment
        assert_eq!(epoch_commitments.len(), 1);

        let slot = epoch_commitments[0].last_slot();
        let chainstate_raw = self
            .cl_client
            .get_chainstate_raw(slot)
            .await
            .inspect_err(|_| error!(%slot, "Failed to fetch raw chainstate"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;

        let chainstate: Chainstate =
            borsh::from_slice(&chainstate_raw).expect("Invalid chainstate from RPC");

        tx_filters.update_from_chainstate(&chainstate);

        Ok(tx_filters)
    }
}

pub(crate) struct BtcBlockscanParams {
    pub range: (L1BlockCommitment, L1BlockCommitment),
    pub epoch: Epoch,
}

impl ProvingOp for BtcBlockspaceOperator {
    type Program = BtcBlockspaceProgram;

    type Params = BtcBlockscanParams;

    fn construct_proof_ctx(
        &self,
        btc_params: &Self::Params,
    ) -> Result<ProofContext, ProvingTaskError> {
        let BtcBlockscanParams { epoch, range } = btc_params;
        let (start, end) = range;
        // Do some sanity checks
        assert!(
            end.height() >= start.height(),
            "failed to construct Btc blockspace proof context. start_height: {} > end_height {}",
            start.height(),
            end.height()
        );
        Ok(ProofContext::BtcBlockspace(*epoch, *start, *end))
    }

    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        _db: &ProofDb,
    ) -> Result<BlockScanProofInput, ProvingTaskError> {
        let (epoch, start, end) = match task_id.context() {
            ProofContext::BtcBlockspace(epoch, start, end) => (epoch, *start, *end),
            _ => return Err(ProvingTaskError::InvalidInput("BtcBlockspace".to_string())),
        };

        let mut btc_blocks = vec![];
        let mut current_block_id = *end.blkid();
        loop {
            let btc_block = self
                .btc_client
                .get_block(&current_block_id.into())
                .await
                .inspect_err(|_| error!(%current_block_id, "Failed to fetch BTC BlockId"))
                .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;

            let prev_block_hash = btc_block.header.prev_blockhash;

            btc_blocks.push(btc_block);

            if start.blkid() == &current_block_id {
                break;
            } else {
                current_block_id = prev_block_hash.into();
            }
        }

        // Reverse the blocks to make them in ascending order
        btc_blocks.reverse();

        let tx_filters = self.construct_tx_filter_config(*epoch).await?;

        Ok(BlockScanProofInput {
            btc_blocks,
            tx_filters,
        })
    }
}
