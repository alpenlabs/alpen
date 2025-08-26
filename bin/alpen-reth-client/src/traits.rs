use async_trait::async_trait;

use crate::{
    errors::OlClientError,
    types::{
        AccountId, AccountStateCommitment, EEAccount, L1BlockCommitment, L1BlockId,
        OlBlockCommitment, OlBlockId,
    },
};

#[async_trait]
pub trait OlClient {
    async fn account_state_commitment_at(
        &self,
        account_id: &AccountId,
        blockid: &OlBlockId,
    ) -> eyre::Result<AccountStateCommitment>;

    async fn best_ol_block(&self) -> eyre::Result<OlBlockCommitment>;

    async fn ol_block_for_l1(&self, l1block: &L1BlockCommitment)
        -> eyre::Result<OlBlockCommitment>;

    async fn submit_account_update(
        &self,
        account_id: &AccountId,
        update: &EEAccount,
    ) -> eyre::Result<()>;
}

#[async_trait]
pub trait L1Client {
    async fn get_l1_commitment(&self, blockhash: L1BlockId) -> eyre::Result<L1BlockCommitment>;

    async fn get_l1_commitment_by_height(&self, height: u64) -> eyre::Result<L1BlockCommitment>;
}

/// for rpc temporarily; for p2p this will be different
#[async_trait]
pub trait ELSequencerClient {
    async fn get_latest_state_commitment(&self) -> eyre::Result<AccountStateCommitment>;
}
