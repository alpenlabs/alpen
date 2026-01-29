//! OL client wrapper that supports both real RPC and dummy implementations.

use alpen_ee_common::{
    OLAccountStateView, OLBlockData, OLChainStatus, OLClient, OLClientError, OLEpochSummary,
    SequencerOLClient,
};
use async_trait::async_trait;
use strata_identifiers::Epoch;
use strata_snark_acct_types::SnarkAccountUpdate;

use crate::{dummy_ol_client::DummyOLClient, rpc_client::RpcOLClient};

/// Enum wrapper that can hold either a real RPC client or a dummy client.
///
/// This allows runtime selection between the two client types while maintaining
/// the required trait bounds for use with the EE components.
#[derive(Debug)]
pub(crate) enum OLClientKind {
    /// Real RPC client connecting to an OL node.
    Rpc(RpcOLClient),
    /// Dummy client for testing without an OL node.
    Dummy(DummyOLClient),
}

#[async_trait]
impl OLClient for OLClientKind {
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError> {
        match self {
            Self::Rpc(client) => <RpcOLClient as OLClient>::chain_status(client).await,
            Self::Dummy(client) => <DummyOLClient as OLClient>::chain_status(client).await,
        }
    }

    async fn epoch_summary(&self, epoch: Epoch) -> Result<OLEpochSummary, OLClientError> {
        match self {
            Self::Rpc(client) => <RpcOLClient as OLClient>::epoch_summary(client, epoch).await,
            Self::Dummy(client) => <DummyOLClient as OLClient>::epoch_summary(client, epoch).await,
        }
    }
}

#[async_trait]
impl SequencerOLClient for OLClientKind {
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError> {
        match self {
            Self::Rpc(client) => <RpcOLClient as SequencerOLClient>::chain_status(client).await,
            Self::Dummy(client) => <DummyOLClient as SequencerOLClient>::chain_status(client).await,
        }
    }

    async fn get_inbox_messages(
        &self,
        min_slot: u64,
        max_slot: u64,
    ) -> Result<Vec<OLBlockData>, OLClientError> {
        match self {
            Self::Rpc(client) => client.get_inbox_messages(min_slot, max_slot).await,
            Self::Dummy(client) => client.get_inbox_messages(min_slot, max_slot).await,
        }
    }

    async fn get_latest_account_state(&self) -> Result<OLAccountStateView, OLClientError> {
        match self {
            Self::Rpc(client) => client.get_latest_account_state().await,
            Self::Dummy(client) => client.get_latest_account_state().await,
        }
    }

    async fn submit_update(&self, update: SnarkAccountUpdate) -> Result<(), OLClientError> {
        match self {
            Self::Rpc(client) => client.submit_update(update).await,
            Self::Dummy(client) => client.submit_update(update).await,
        }
    }
}
