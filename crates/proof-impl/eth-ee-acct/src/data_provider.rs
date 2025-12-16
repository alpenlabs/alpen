use strata_acct_types::{AccountId, Hash};
use strata_ee_acct_types::{CommitChainSegment, EeAccountState};
use strata_snark_acct_types::UpdateOperationData;

use crate::program::EthEeAcctInput;

pub type UpdateId = u64;

#[derive(Debug, thiserror::Error)]
pub enum DataProviderError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Account not found: {0:?}")]
    AccountNotFound(AccountId),
    #[error("Update not found: {0}")]
    UpdateNotFound(UpdateId),
    #[error("Block not found: {0:?}")]
    BlockNotFound(Hash),
    #[error("Encoding error: {0}")]
    Encoding(String),
    #[error("Invalid data: {0}")]
    InvalidData(String),
    #[error("SSZ encoding error: {0}")]
    SszEncoding(String),
}

pub type Result<T> = std::result::Result<T, DataProviderError>;

/// Trait for providing all data needed for ETH-EE account proof generation
/// This trait is ONLY for data fetching - proof input assembly is separate
pub trait EthEeAcctDataProvider: Send + Sync {
    /// Fetch the current state of an EE account
    fn fetch_ee_account_state(&self, account_id: AccountId) -> Result<EeAccountState>;

    /// Fetch the update operation data
    fn fetch_update_operation(&self, update_id: UpdateId) -> Result<UpdateOperationData>;

    /// Fetch chain segments for the given commit block IDs
    fn fetch_chain_segments(&self, commit_blkids: &[Hash]) -> Result<Vec<CommitChainSegment>>;

    /// Fetch the encoded previous execution header
    fn fetch_previous_header(&self, exec_blkid: Hash) -> Result<Vec<u8>>;

    /// Fetch or reconstruct the partial state at a given execution block
    fn fetch_partial_state(&self, exec_blkid: Hash) -> Result<Vec<u8>>;
}

/// Assemble proof input from fetched data
/// This function uses the data provider to fetch all required data,
/// then serializes it to SSZ bytes for passing through zkVM
pub fn prepare_proof_input(
    provider: &impl EthEeAcctDataProvider,
    update_id: UpdateId,
    genesis: rsp_primitives::genesis::Genesis,
) -> Result<EthEeAcctInput> {
    // 1. Fetch the update operation
    let operation = provider.fetch_update_operation(update_id)?;

    // 2. Extract account ID and fetch account state
    let account_id = extract_account_id(&operation)?;
    let astate = provider.fetch_ee_account_state(account_id)?;

    // 3. Extract commit block IDs and fetch chain segments
    let commit_blkids = extract_commit_blkids(&operation)?;
    let chain_segments = provider.fetch_chain_segments(&commit_blkids)?;

    // 4. Fetch previous execution state
    let prev_blkid = astate.last_exec_blkid();
    let raw_prev_header = provider.fetch_previous_header(prev_blkid)?;
    let raw_partial_pre_state = provider.fetch_partial_state(prev_blkid)?;

    // 5. Serialize to SSZ bytes
    // TODO: Replace with actual SSZ encoding once available
    let astate_ssz = encode_ssz_placeholder(&astate)?;
    let operation_ssz = encode_ssz_placeholder(&operation)?;
    let commit_segments_ssz = chain_segments
        .iter()
        .map(encode_ssz_placeholder)
        .collect::<Result<Vec<_>>>()?;

    // 6. Handle coinputs for messages
    // TODO: Implement coinputs for messages
    // Currently empty per update_processing.rs:179
    // Each message may need associated witness data
    let message_count = operation.processed_messages().len();
    let coinputs = vec![vec![]; message_count];

    Ok(EthEeAcctInput {
        astate_ssz,
        operation_ssz,
        coinputs,
        commit_segments_ssz,
        raw_prev_header,
        raw_partial_pre_state,
        genesis,
    })
}

/// Extract account ID from update operation
/// TODO: Implement based on actual UpdateOperationData structure
fn extract_account_id(_operation: &UpdateOperationData) -> Result<AccountId> {
    Err(DataProviderError::InvalidData(
        "extract_account_id not yet implemented".to_string(),
    ))
}

/// Extract commit block IDs from update operation extra data
/// TODO: Implement by parsing UpdateExtraData from operation.extra_data()
fn extract_commit_blkids(_operation: &UpdateOperationData) -> Result<Vec<Hash>> {
    Err(DataProviderError::InvalidData(
        "extract_commit_blkids not yet implemented".to_string(),
    ))
}

/// Placeholder for SSZ encoding until proper SSZ support is available
/// TODO: Replace with actual SSZ encoding (coordinate with Dilli)
fn encode_ssz_placeholder<T>(_value: &T) -> Result<Vec<u8>> {
    Err(DataProviderError::SszEncoding(
        "SSZ encoding not yet implemented - coordinate with the team for EeAccountState, UpdateOperationData, and CommitChainSegment SSZ support".to_string(),
    ))
}
