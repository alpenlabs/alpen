//! Block assembly context traits and implementation.

use std::{
    fmt::{self, Debug, Display},
    sync::Arc,
};

use async_trait::async_trait;
use strata_acct_types::{AccountId, tree_hash::TreeHash};
use strata_asm_manifest_types::AsmManifest;
use strata_db_types::{errors::DbError, mmr_helpers::leaf_index_to_pos};
use strata_identifiers::{Hash, MmrId, OLBlockCommitment, OLBlockId, OLTxId};
use strata_ledger_types::{IAccountStateConstructible, IAccountStateMut, IStateAccessor};
use strata_ol_chain_types_new::OLBlock;
use strata_ol_mempool::{MempoolTxInvalidReason, OLMempoolTransaction};
use strata_ol_state_types::{IStateBatchApplicable, StateProvider};
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefProofs, MessageEntry, MessageEntryProof, MmrEntryProof,
};
use strata_storage::NodeStorage;

use crate::{BlockAssemblyError, BlockAssemblyResult, MempoolProvider};

/// Account state capabilities required by block assembly.
pub trait BlockAssemblyAccountState:
    Clone + IAccountStateConstructible + IAccountStateMut + Send + Sync
{
}

impl<T> BlockAssemblyAccountState for T where
    T: Clone + IAccountStateConstructible + IAccountStateMut + Send + Sync
{
}

/// State capabilities required by block assembly.
pub trait BlockAssemblyStateAccess:
    IStateBatchApplicable
    + IStateAccessor<AccountState: BlockAssemblyAccountState>
    + Clone
    + Send
    + Sync
{
}

impl<T> BlockAssemblyStateAccess for T where
    T: IStateBatchApplicable
        + IStateAccessor<AccountState: BlockAssemblyAccountState>
        + Clone
        + Send
        + Sync
{
}

/// Anchoring inputs needed by block assembly.
///
/// Provides access to the parent OL block, state, and ASM manifests needed for block construction.
#[async_trait]
pub trait BlockAssemblyAnchorContext: Send + Sync + 'static {
    type State: BlockAssemblyStateAccess;

    /// Fetch an OL block by ID.
    async fn fetch_ol_block(&self, id: OLBlockId) -> BlockAssemblyResult<Option<OLBlock>>;

    /// Fetch the state snapshot for `tip`.
    async fn fetch_state_for_tip(
        &self,
        tip: OLBlockCommitment,
    ) -> BlockAssemblyResult<Option<Arc<Self::State>>>;

    /// Fetch ASM manifests from `start_height` to latest (ascending).
    async fn fetch_asm_manifests_from(
        &self,
        start_height: u64,
    ) -> BlockAssemblyResult<Vec<AsmManifest>>;
}

/// Generates MMR proofs needed during block assembly.
pub trait AccumulatorProofGenerator: Send + Sync + 'static {
    /// Validates inbox message indices and generates message entry proofs.
    fn generate_inbox_proofs(
        &self,
        target: AccountId,
        messages: &[MessageEntry],
        start_idx: u64,
    ) -> BlockAssemblyResult<Vec<MessageEntryProof>>;

    /// Validates claims and generates L1 header reference proofs.
    fn generate_l1_header_proofs(
        &self,
        l1_header_refs: &[AccumulatorClaim],
    ) -> BlockAssemblyResult<LedgerRefProofs>;
}

/// Concrete context passed to block assembly.
///
/// Implements:
/// - [`BlockAssemblyAnchorContext`]
/// - [`MempoolProvider`]
/// - [`AccumulatorProofGenerator`]
#[derive(Clone)]
pub struct BlockAssemblyContext<M, S> {
    storage: Arc<NodeStorage>,
    mempool_provider: M,
    state_provider: Arc<S>,
}

impl<M, S> Debug for BlockAssemblyContext<M, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockAssemblyContext")
            .field("storage", &"<NodeStorage>")
            .finish_non_exhaustive()
    }
}

impl<M, S> BlockAssemblyContext<M, S> {
    /// Create a new block assembly context.
    pub fn new(storage: Arc<NodeStorage>, mempool_provider: M, state_provider: Arc<S>) -> Self {
        Self {
            storage,
            mempool_provider,
            state_provider,
        }
    }

    fn validate_l1_header_claims(
        &self,
        l1_header_refs: &[AccumulatorClaim],
    ) -> BlockAssemblyResult<()> {
        let mmr_handle = self.storage.global_mmr().as_ref().get_handle(MmrId::Asm);
        for claim in l1_header_refs {
            let leaf_idx = claim.idx();
            let pos = leaf_index_to_pos(leaf_idx);

            let entry_hash = claim.entry_hash();
            let actual_hash = mmr_handle
                .get_node_blocking(pos)
                .map_err(map_l1_header_mmr_error)?
                .ok_or(BlockAssemblyError::L1HeaderLeafNotFound(leaf_idx))?;

            if actual_hash.as_ref() != entry_hash.as_ref() {
                return Err(BlockAssemblyError::L1HeaderHashMismatch {
                    idx: leaf_idx,
                    expected: entry_hash,
                    actual: actual_hash,
                });
            }
        }

        Ok(())
    }

    fn validate_inbox_entries(
        &self,
        target: AccountId,
        messages: &[MessageEntry],
        start_idx: u64,
    ) -> BlockAssemblyResult<()> {
        let mmr_handle = self
            .storage
            .global_mmr()
            .as_ref()
            .get_handle(MmrId::SnarkMsgInbox(target));
        for (offset, message) in messages.iter().enumerate() {
            let idx = start_idx + offset as u64;
            let pos = leaf_index_to_pos(idx);
            let expected_hash: Hash = <MessageEntry as TreeHash>::tree_hash_root(message).into();
            let actual_hash = mmr_handle
                .get_node_blocking(pos)
                .map_err(|e| map_inbox_mmr_error(e, target))?
                .ok_or(BlockAssemblyError::InboxLeafNotFound {
                    idx,
                    account_id: target,
                })?;

            if actual_hash.as_ref() != expected_hash.as_ref() {
                return Err(BlockAssemblyError::InboxEntryHashMismatch {
                    idx,
                    account_id: target,
                    expected: expected_hash,
                    actual: actual_hash,
                });
            }
        }

        Ok(())
    }
}

#[async_trait]
impl<M, S> BlockAssemblyAnchorContext for BlockAssemblyContext<M, S>
where
    M: Send + Sync + 'static,
    S: StateProvider + Send + Sync + 'static,
    S::Error: Display,
    S::State: BlockAssemblyStateAccess,
{
    type State = <S as StateProvider>::State;

    async fn fetch_ol_block(&self, id: OLBlockId) -> BlockAssemblyResult<Option<OLBlock>> {
        self.storage
            .ol_block()
            .get_block_data_async(id)
            .await
            .map_err(BlockAssemblyError::Database)
    }

    async fn fetch_state_for_tip(
        &self,
        tip: OLBlockCommitment,
    ) -> BlockAssemblyResult<Option<Arc<Self::State>>> {
        self.state_provider
            .get_state_for_tip_async(tip)
            .await
            // keep current logic: stringified provider error
            .map_err(|e| BlockAssemblyError::Other(e.to_string()))
    }

    async fn fetch_asm_manifests_from(
        &self,
        start_height: u64,
    ) -> BlockAssemblyResult<Vec<AsmManifest>> {
        let end_height = match self
            .storage
            .asm()
            .fetch_most_recent_state()
            .map_err(BlockAssemblyError::Database)?
        {
            Some((commitment, _)) => commitment.height_u64(),
            None => return Ok(Vec::new()),
        };

        if start_height > end_height {
            return Ok(Vec::new());
        }

        let mut manifests = Vec::new();
        for height in start_height..=end_height {
            let manifest = self
                .storage
                .l1()
                .get_block_manifest_at_height_async(height)
                .await
                .map_err(BlockAssemblyError::Database)?
                .ok_or_else(|| {
                    BlockAssemblyError::Database(DbError::Other(format!(
                        "L1 block manifest not found at height {height}"
                    )))
                })?;
            manifests.push(manifest);
        }

        Ok(manifests)
    }
}

#[async_trait]
impl<M, S> MempoolProvider for BlockAssemblyContext<M, S>
where
    M: MempoolProvider + Send + Sync + 'static,
    S: Send + Sync + 'static,
{
    async fn get_transactions(
        &self,
        limit: usize,
    ) -> BlockAssemblyResult<Vec<(OLTxId, OLMempoolTransaction)>> {
        MempoolProvider::get_transactions(&self.mempool_provider, limit).await
    }

    async fn report_invalid_transactions(
        &self,
        txs: &[(OLTxId, MempoolTxInvalidReason)],
    ) -> BlockAssemblyResult<()> {
        MempoolProvider::report_invalid_transactions(&self.mempool_provider, txs).await
    }
}

/// Convert MMR-related database errors to appropriate block assembly errors for L1 header proofs.
fn map_l1_header_mmr_error(e: DbError) -> BlockAssemblyError {
    match e {
        DbError::MmrLeafNotFound(idx) => BlockAssemblyError::L1HeaderLeafNotFound(idx),
        DbError::MmrInvalidRange { start, end } => {
            BlockAssemblyError::InvalidMmrRange { start, end }
        }
        other => BlockAssemblyError::Database(other),
    }
}

/// Convert MMR-related database errors to appropriate block assembly errors for inbox proofs.
fn map_inbox_mmr_error(e: DbError, account_id: AccountId) -> BlockAssemblyError {
    match e {
        DbError::MmrLeafNotFound(idx) => BlockAssemblyError::InboxLeafNotFound { idx, account_id },
        DbError::MmrLeafNotFoundForAccount(idx, account_id) => {
            BlockAssemblyError::InboxLeafNotFound { idx, account_id }
        }
        DbError::MmrInvalidRange { start, end } => {
            BlockAssemblyError::InvalidMmrRange { start, end }
        }
        other => BlockAssemblyError::Database(other),
    }
}

impl<M, S> AccumulatorProofGenerator for BlockAssemblyContext<M, S>
where
    M: Send + Sync + 'static,
    S: Send + Sync + 'static,
{
    fn generate_inbox_proofs(
        &self,
        target: AccountId,
        messages: &[MessageEntry],
        start_idx: u64,
    ) -> BlockAssemblyResult<Vec<MessageEntryProof>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }

        // Get MMR handle for this account's inbox
        let mmr_handle = self
            .storage
            .global_mmr()
            .as_ref()
            .get_handle(MmrId::SnarkMsgInbox(target));

        self.validate_inbox_entries(target, messages, start_idx)?;

        // Generate proofs for the range of messages (end index is inclusive).
        let end_idx = start_idx + messages.len() as u64 - 1;
        let merkle_proofs = mmr_handle
            .generate_proofs(start_idx, end_idx)
            .map_err(|e| map_inbox_mmr_error(e, target))?;

        // Verify we got the expected number of proofs
        if merkle_proofs.len() != messages.len() {
            return Err(BlockAssemblyError::InboxProofCountMismatch {
                expected: messages.len(),
                got: merkle_proofs.len(),
            });
        }

        // Build MessageEntryProof for each message
        let inbox_proofs = messages
            .iter()
            .zip(merkle_proofs)
            .map(|(message, merkle_proof)| {
                let raw_proof = merkle_proof.inner.clone();
                MessageEntryProof::new(message.clone(), raw_proof)
            })
            .collect();

        Ok(inbox_proofs)
    }

    fn generate_l1_header_proofs(
        &self,
        l1_header_refs: &[AccumulatorClaim],
    ) -> BlockAssemblyResult<LedgerRefProofs> {
        let mmr_handle = self.storage.global_mmr().as_ref().get_handle(MmrId::Asm);

        self.validate_l1_header_claims(l1_header_refs)?;

        // Generate proofs
        let mut l1_header_proofs = Vec::new();
        for claim in l1_header_refs {
            let entry_hash: [u8; 32] = claim.entry_hash().into();

            let merkle_proof = mmr_handle
                .generate_proof(claim.idx())
                .map_err(map_l1_header_mmr_error)?;

            let mmr_proof = MmrEntryProof::new(entry_hash, merkle_proof);
            l1_header_proofs.push(mmr_proof);
        }

        Ok(LedgerRefProofs::new(l1_header_proofs))
    }
}

#[cfg(test)]
mod tests {
    use strata_snark_acct_types::AccumulatorClaim;

    use super::*;
    use crate::test_utils::{
        StorageAsmMmr, StorageInboxMmr, create_test_context, create_test_message,
        create_test_storage, test_account_id, test_hash,
    };

    // =========================================================================
    // L1 Header Proof Generation Tests
    // =========================================================================

    #[test]
    fn test_l1_header_proof_gen_success() {
        let storage = create_test_storage();

        // Add a header hash to the ASM MMR
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_header(test_hash(42));

        // Collect claims and hashes before creating context
        let claims = asm_mmr.claims();
        let expected_hash = asm_mmr.hashes()[0];

        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&claims);

        assert!(result.is_ok(), "Should succeed with valid claim");
        let proofs = result.unwrap();
        assert_eq!(proofs.l1_headers_proofs().len(), 1);
        assert_eq!(proofs.l1_headers_proofs()[0].entry_hash(), expected_hash);
    }

    #[test]
    fn test_l1_header_proof_gen_multiple_claims() {
        let storage = create_test_storage();

        // Add multiple header hashes
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_headers((1..=3).map(test_hash));

        let claims = asm_mmr.claims();

        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&claims);

        assert!(result.is_ok(), "Should succeed with multiple valid claims");
        let proofs = result.unwrap();
        assert_eq!(proofs.l1_headers_proofs().len(), 3);
    }

    #[test]
    fn test_l1_header_proof_gen_hash_mismatch() {
        let storage = create_test_storage();

        // Add a header hash to the ASM MMR
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_header(test_hash(42));

        // Create claim with correct index but wrong hash
        let claim_idx = asm_mmr.indices()[0];
        let wrong_hash = test_hash(99);
        let claim = AccumulatorClaim::new(claim_idx, wrong_hash);

        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&[claim]);

        assert!(result.is_err(), "Should fail with hash mismatch");
        let err = result.unwrap_err();
        assert!(
            matches!(&err, BlockAssemblyError::L1HeaderHashMismatch { idx, .. } if *idx == claim_idx),
            "Expected L1HeaderHashMismatch error, got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_proof_gen_missing_index() {
        let storage = create_test_storage();

        // Add one header but request a different index
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_header(test_hash(42));

        // Create claim with non-existent index (index 999 doesn't exist)
        let nonexistent_index = 999u64;
        let claim = AccumulatorClaim::new(nonexistent_index, asm_mmr.hashes()[0]);

        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&[claim]);

        assert!(result.is_err(), "Should fail with missing index");
        let err = result.unwrap_err();
        assert!(
            matches!(&err, BlockAssemblyError::L1HeaderLeafNotFound(idx) if *idx == nonexistent_index),
            "Expected L1HeaderLeafNotFound error, got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_claim_empty_mmr() {
        let storage = create_test_storage();
        let claim = AccumulatorClaim::new(0, test_hash(42));
        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&[claim]);

        assert!(result.is_err(), "Should fail when MMR is empty");
        let err = result.unwrap_err();
        assert!(
            matches!(err, BlockAssemblyError::L1HeaderLeafNotFound(0)),
            "Expected L1HeaderLeafNotFound, got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_proof_gen_empty_claims() {
        let storage = create_test_storage();
        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&[]);

        assert!(result.is_ok(), "Should succeed with empty claims");
        let proofs = result.unwrap();
        assert!(proofs.l1_headers_proofs().is_empty());
    }

    // =========================================================================
    // Inbox Proof Generation Tests
    // =========================================================================

    #[test]
    fn test_inbox_proof_gen_success() {
        let storage = create_test_storage();
        let account_id = test_account_id(1);

        // Add messages to the inbox MMR using the tracker
        let mut inbox_mmr = StorageInboxMmr::new(&storage, account_id);
        let messages: Vec<_> = (1..=2)
            .map(|i| create_test_message(i, i as u32, 1000 * i as u64))
            .collect();
        inbox_mmr.add_messages(messages);

        // Collect entries before creating context
        let entries: Vec<_> = inbox_mmr.entries().to_vec();

        let ctx = create_test_context(storage);

        let result = ctx.generate_inbox_proofs(account_id, &entries, 0);

        assert!(
            result.is_ok(),
            "Should succeed with valid messages, got: {:?}",
            result.err()
        );
        let proofs = result.unwrap();
        assert_eq!(proofs.len(), 2);
        assert_eq!(proofs[0].entry(), &entries[0]);
        assert_eq!(proofs[1].entry(), &entries[1]);
    }

    #[test]
    fn test_inbox_proof_gen_empty_messages() {
        let storage = create_test_storage();
        let account_id = test_account_id(1);

        let ctx = create_test_context(storage);

        let result = ctx.generate_inbox_proofs(account_id, &[], 0);

        assert!(result.is_ok(), "Should succeed with empty messages");
        let proofs = result.unwrap();
        assert!(proofs.is_empty());
    }

    #[test]
    fn test_inbox_proof_gen_with_offset() {
        let storage = create_test_storage();
        let account_id = test_account_id(1);

        // Add 4 messages to the inbox MMR using the tracker
        let mut inbox_mmr = StorageInboxMmr::new(&storage, account_id);
        let all_messages: Vec<_> = (1..=4)
            .map(|i| create_test_message(i, i as u32, 1000 * i as u64))
            .collect();
        inbox_mmr.add_messages(all_messages);

        // Collect entries before creating context
        let entries: Vec<_> = inbox_mmr.entries().to_vec();

        let ctx = create_test_context(storage);

        // Request proofs starting at index 2 for last 2 messages
        let messages_to_prove = &entries[2..];
        let result = ctx.generate_inbox_proofs(account_id, messages_to_prove, 2);

        assert!(
            result.is_ok(),
            "Should succeed with offset, got: {:?}",
            result.err()
        );
        let proofs = result.unwrap();
        assert_eq!(proofs.len(), 2);
        assert_eq!(proofs[0].entry(), &entries[2]);
        assert_eq!(proofs[1].entry(), &entries[3]);
    }

    #[test]
    fn test_inbox_proof_gen_missing_messages() {
        let storage = create_test_storage();
        let account_id = test_account_id(1);

        // Don't add any messages to MMR, but try to generate proofs
        let ctx = create_test_context(storage);

        let messages = vec![create_test_message(1, 1, 1000)];
        let result = ctx.generate_inbox_proofs(account_id, &messages, 0);

        assert!(result.is_err(), "Should fail when MMR has no messages");
    }

    #[test]
    fn test_inbox_claim_missing_index() {
        let storage = create_test_storage();
        let account_id = test_account_id(1);

        // Add one message at index 0
        let mut inbox_mmr = StorageInboxMmr::new(&storage, account_id);
        let stored_message = create_test_message(1, 1, 1000);
        inbox_mmr.add_message(stored_message);

        // Claim messages starting at a non-existent index
        let claimed_messages = vec![create_test_message(2, 2, 2000)];
        let ctx = create_test_context(storage);

        let result = ctx.generate_inbox_proofs(account_id, &claimed_messages, 5);

        assert!(result.is_err(), "Should fail for missing inbox index");
        let err = result.unwrap_err();
        assert!(
            matches!(err, BlockAssemblyError::InboxLeafNotFound { .. }),
            "Expected InboxLeafNotFound, got: {:?}",
            err
        );
    }

    #[test]
    fn test_inbox_claim_hash_mismatch() {
        let storage = create_test_storage();
        let account_id = test_account_id(1);

        // Add one message at index 0
        let mut inbox_mmr = StorageInboxMmr::new(&storage, account_id);
        let stored_message = create_test_message(1, 1, 1000);
        inbox_mmr.add_message(stored_message);

        // Claim different message for the same index
        let claimed_messages = vec![create_test_message(2, 2, 2000)];
        let ctx = create_test_context(storage);

        let result = ctx.generate_inbox_proofs(account_id, &claimed_messages, 0);

        assert!(
            result.is_err(),
            "Should fail for mismatched inbox entry hash"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, BlockAssemblyError::InboxEntryHashMismatch { .. }),
            "Expected InboxEntryHashMismatch, got: {:?}",
            err
        );
    }
}
