//! Block assembly context traits and implementation.

use std::{
    fmt::{self, Debug, Display},
    sync::Arc,
};

use async_trait::async_trait;
use strata_acct_types::{AccountId, tree_hash::TreeHash};
use strata_asm_manifest_types::AsmManifest;
use strata_db_types::{MmrId, errors::DbError};
use strata_identifiers::{Hash, L1Height, OLBlockCommitment, OLBlockId, OLTxId};
use strata_ledger_types::{
    IAccountStateConstructible, IAccountStateMut, IStateAccessor, asm_manifests_mmr_start_height,
};
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
        start_height: L1Height,
    ) -> BlockAssemblyResult<Vec<AsmManifest>>;
}

/// Generates MMR proofs needed during block assembly.
pub trait AccumulatorProofGenerator: Send + Sync + 'static {
    /// Generates inbox message entry proofs at `at_leaf_count`.
    fn generate_inbox_proofs_at(
        &self,
        target: AccountId,
        messages: &[MessageEntry],
        start_idx: u64,
        at_leaf_count: u64,
    ) -> BlockAssemblyResult<Vec<MessageEntryProof>>;

    /// Validates claims and generates L1 header reference proofs.
    fn generate_l1_header_proofs<T: IStateAccessor>(
        &self,
        l1_header_refs: &[AccumulatorClaim],
        state: &T,
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
    state_provider: S,
    _genesis_l1_height: L1Height,
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
    pub fn new(
        storage: Arc<NodeStorage>,
        mempool_provider: M,
        state_provider: S,
        genesis_l1_height: L1Height,
    ) -> Self {
        Self {
            storage,
            mempool_provider,
            state_provider,
            _genesis_l1_height: genesis_l1_height,
        }
    }

    /// Converts an L1 block height to an MMR leaf index using the given start height.
    fn height_to_mmr_index(&self, height: u64, mmr_start_height: u64) -> BlockAssemblyResult<u64> {
        let offset = mmr_start_height;
        height.checked_sub(offset).ok_or_else(|| {
            BlockAssemblyError::Other(format!(
                "L1 height {height} is before MMR start offset {offset}"
            ))
        })
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
            .map_err(BlockAssemblyError::Db)
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
        start_height: L1Height,
    ) -> BlockAssemblyResult<Vec<AsmManifest>> {
        let end_height = match self
            .storage
            .asm()
            .fetch_most_recent_state()
            .map_err(BlockAssemblyError::Db)?
        {
            Some((commitment, _)) => commitment.height(),
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
                .map_err(BlockAssemblyError::Db)?
                .ok_or_else(|| {
                    BlockAssemblyError::Db(DbError::Other(format!(
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

impl<M, S> AccumulatorProofGenerator for BlockAssemblyContext<M, S>
where
    M: Send + Sync + 'static,
    S: Send + Sync + 'static,
{
    fn generate_inbox_proofs_at(
        &self,
        target: AccountId,
        messages: &[MessageEntry],
        start_idx: u64,
        at_leaf_count: u64,
    ) -> BlockAssemblyResult<Vec<MessageEntryProof>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }

        let mmr_handle = self
            .storage
            .mmr_index()
            .as_ref()
            .get_handle(MmrId::SnarkMsgInbox(target));
        let expected_hashes: Vec<Hash> = messages
            .iter()
            .map(|message| <MessageEntry as TreeHash>::tree_hash_root(message).into())
            .collect();
        let merkle_proofs = mmr_handle
            .generate_proofs_for(start_idx, &expected_hashes, at_leaf_count)
            .map_err(|err| match err {
                DbError::MmrLeafHashMismatch { idx, expected, got } => {
                    BlockAssemblyError::InboxEntryHashMismatch {
                        idx,
                        account_id: target,
                        expected,
                        actual: got,
                    }
                }
                other => BlockAssemblyError::Db(other),
            })?;

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

    fn generate_l1_header_proofs<T: IStateAccessor>(
        &self,
        l1_header_refs: &[AccumulatorClaim],
        state: &T,
    ) -> BlockAssemblyResult<LedgerRefProofs> {
        if l1_header_refs.is_empty() {
            return Ok(LedgerRefProofs::new(Vec::new()));
        }

        let mmr_handle = self.storage.mmr_index().as_ref().get_handle(MmrId::Asm);
        let at_leaf_count = state.asm_manifests_mmr().num_entries();
        let mmr_start_height = asm_manifests_mmr_start_height(state)
            .ok_or_else(|| BlockAssemblyError::Other("invalid manifests MMR start height".into()))?
            as u64;
        let indices_and_hashes = l1_header_refs
            .iter()
            .map(|claim| {
                let mmr_idx = self.height_to_mmr_index(claim.idx(), mmr_start_height)?;
                Ok((mmr_idx, claim.entry_hash()))
            })
            .collect::<BlockAssemblyResult<Vec<_>>>()?;

        let merkle_proofs = mmr_handle
            .generate_proofs_for_indices(&indices_and_hashes, at_leaf_count)
            .map_err(|err| match err {
                DbError::MmrLeafHashMismatch { idx, expected, got } => {
                    BlockAssemblyError::L1HeaderHashMismatch {
                        idx,
                        expected,
                        actual: got,
                    }
                }
                other => BlockAssemblyError::Db(other),
            })?;

        let l1_header_proofs = indices_and_hashes
            .into_iter()
            .zip(merkle_proofs)
            .map(|((_, entry_hash), merkle_proof)| MmrEntryProof::new(entry_hash, merkle_proof))
            .collect();
        Ok(LedgerRefProofs::new(l1_header_proofs))
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_manifest_types::AsmManifest;
    use strata_identifiers::{Buf32, L1BlockId, WtxidsRoot};
    use strata_ledger_types::IStateAccessor;
    use strata_snark_acct_types::AccumulatorClaim;

    use super::*;
    use crate::test_utils::{
        StorageAsmMmr, StorageInboxMmr, create_test_context, create_test_genesis_state,
        create_test_message, create_test_storage, test_account_id, test_hash,
    };

    // =========================================================================
    // L1 Header Proof Generation Tests
    // =========================================================================

    fn create_test_manifest(height: L1Height, seed: u8) -> AsmManifest {
        let mut blkid_bytes = [0u8; 32];
        blkid_bytes[0] = seed;
        AsmManifest::new(
            height,
            L1BlockId::from(Buf32::from(blkid_bytes)),
            WtxidsRoot::from(Buf32::zero()),
            vec![],
        )
    }

    #[test]
    fn test_l1_header_proof_gen_success() {
        let storage = create_test_storage();
        let manifest = create_test_manifest(1, 1);
        let manifest_hash = manifest.compute_hash().into();

        // Add a header hash to the ASM MMR
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_header(manifest_hash);

        // Collect claims and hashes before creating context
        let claims = asm_mmr.claims(0);
        let expected_hash = asm_mmr.hashes()[0];
        let mut state = create_test_genesis_state();
        state.append_manifest(manifest.height(), manifest);

        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&claims, &state);

        assert!(result.is_ok(), "Should succeed with valid claim");
        let proofs = result.unwrap();
        assert_eq!(proofs.l1_headers_proofs().len(), 1);
        assert_eq!(proofs.l1_headers_proofs()[0].entry_hash(), expected_hash);
    }

    #[test]
    fn test_l1_header_proof_gen_multiple_claims() {
        let storage = create_test_storage();
        let manifests = vec![
            create_test_manifest(1, 1),
            create_test_manifest(2, 2),
            create_test_manifest(3, 3),
        ];
        let manifest_hashes: Vec<Hash> = manifests
            .iter()
            .map(|mf| mf.compute_hash().into())
            .collect();

        // Add multiple header hashes
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_headers(manifest_hashes.iter().copied());

        let claims = asm_mmr.claims(0);
        let mut state = create_test_genesis_state();
        for manifest in manifests {
            state.append_manifest(manifest.height(), manifest);
        }

        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&claims, &state);

        assert!(result.is_ok(), "Should succeed with multiple valid claims");
        let proofs = result.unwrap();
        assert_eq!(proofs.l1_headers_proofs().len(), 3);
    }

    #[test]
    fn test_l1_header_proof_gen_hash_mismatch() {
        let storage = create_test_storage();
        let manifest = create_test_manifest(1, 1);
        let manifest_hash = manifest.compute_hash().into();

        // Add a header hash to the ASM MMR
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_header(manifest_hash);

        // Create claim with correct height but wrong hash
        let claim_height = asm_mmr.indices()[0] + 1; // height = mmr_idx + offset(1)
        let wrong_hash = test_hash(99);
        let claim = AccumulatorClaim::new(claim_height, wrong_hash);
        let expected_hash = asm_mmr.hashes()[0];
        let mut state = create_test_genesis_state();
        state.append_manifest(manifest.height(), manifest);

        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&[claim], &state);

        assert!(
            result.is_err(),
            "Should fail when claim hash does not match MMR leaf"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                BlockAssemblyError::L1HeaderHashMismatch {
                    idx: 0,
                    expected,
                    actual
                } if expected == wrong_hash && actual == expected_hash
            ),
            "Expected L1HeaderHashMismatch, got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_proof_gen_missing_index() {
        let storage = create_test_storage();
        let manifest = create_test_manifest(1, 1);
        let manifest_hash = manifest.compute_hash().into();

        // Add one header but request a different index
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_header(manifest_hash);

        // Create claim with non-existent height (height 999 → MMR index 998, doesn't exist)
        let nonexistent_height = 999u64;
        let claim = AccumulatorClaim::new(nonexistent_height, asm_mmr.hashes()[0]);
        let mut state = create_test_genesis_state();
        state.append_manifest(manifest.height(), manifest);

        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&[claim], &state);

        assert!(result.is_err(), "Should fail with missing index");
        let err = result.unwrap_err();
        let expected_mmr_idx = nonexistent_height - 1; // offset = genesis_height(0) + 1
        assert!(
            matches!(
                &err,
                BlockAssemblyError::Db(DbError::MmrIndexOutOfRange { requested, cur })
                    if *requested == expected_mmr_idx && *cur == 1
            ),
            "Expected Db(MmrIndexOutOfRange) error, got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_claim_empty_mmr() {
        let storage = create_test_storage();
        // height=1 is the minimum valid height (offset = genesis_height(0) + 1)
        let claim = AccumulatorClaim::new(1, test_hash(42));
        let state = create_test_genesis_state();
        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&[claim], &state);

        assert!(result.is_err(), "Should fail when MMR is empty");
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                BlockAssemblyError::Db(DbError::MmrIndexOutOfRange {
                    requested: 0,
                    cur: 0
                })
            ),
            "Expected Db(MmrIndexOutOfRange {{ requested: 0, cur: 0 }}), got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_proof_gen_empty_claims() {
        let storage = create_test_storage();
        let state = create_test_genesis_state();
        let ctx = create_test_context(storage);

        let result = ctx.generate_l1_header_proofs(&[], &state);

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

        let result = ctx.generate_inbox_proofs_at(account_id, &entries, 0, entries.len() as u64);

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

        let result = ctx.generate_inbox_proofs_at(account_id, &[], 0, 0);

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
        let result =
            ctx.generate_inbox_proofs_at(account_id, messages_to_prove, 2, entries.len() as u64);

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
        let result = ctx.generate_inbox_proofs_at(account_id, &messages, 0, 0);

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

        let result = ctx.generate_inbox_proofs_at(account_id, &claimed_messages, 5, 1);

        assert!(result.is_err(), "Should fail for missing inbox index");
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                BlockAssemblyError::Db(DbError::MmrIndexOutOfRange { .. })
                    | BlockAssemblyError::Db(DbError::MmrLeafNotFound(_))
            ),
            "Expected Db(MmrIndexOutOfRange|MmrLeafNotFound), got: {:?}",
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

        let result = ctx.generate_inbox_proofs_at(account_id, &claimed_messages, 0, 1);

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
