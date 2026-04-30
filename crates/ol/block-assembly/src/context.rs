//! Block assembly context traits and implementation.

use std::{
    fmt::{self, Debug, Display},
    sync::Arc,
};

use async_trait::async_trait;
use strata_acct_types::{
    AccountId, AccumulatorClaim, MessageEntry, RawMerkleProof, tree_hash::TreeHash,
};
use strata_asm_manifest_types::AsmManifest;
use strata_db_types::{MmrId, errors::DbError};
use strata_identifiers::{Hash, L1Height, OLBlockCommitment, OLBlockId, OLTxId};
use strata_ledger_types::{IAccountState, IAccountStateMut, IStateAccessor, IStateAccessorMut};
use strata_ol_chain_types_new::{OLBlock, OLTransaction};
use strata_ol_mempool::MempoolTxInvalidReason;
use strata_ol_state_provider::StateProvider;
use strata_ol_state_support_types::IComputeStateRootWithWrites;
use strata_ol_state_types::IStateBatchApplicable;
use strata_snark_acct_types::LedgerRefProofs;
use strata_storage::NodeStorage;

use crate::{BlockAssemblyError, BlockAssemblyResult, MempoolProvider};

/// Account state capabilities required by block assembly.
pub trait BlockAssemblyAccountState:
    Clone + IAccountState + IAccountStateMut + Send + Sync
{
}

impl<T> BlockAssemblyAccountState for T where
    T: Clone + IAccountState + IAccountStateMut + Send + Sync
{
}

/// State capabilities required by block assembly.
pub trait BlockAssemblyStateAccess:
    IComputeStateRootWithWrites
    + IStateBatchApplicable
    + IStateAccessorMut<AccountState: BlockAssemblyAccountState>
    + Clone
    + Send
    + Sync
{
}

impl<T> BlockAssemblyStateAccess for T where
    T: IComputeStateRootWithWrites
        + IStateBatchApplicable
        + IStateAccessorMut<AccountState: BlockAssemblyAccountState>
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
    ) -> BlockAssemblyResult<Vec<RawMerkleProof>>;

    /// Generates inbox MMR proofs for the given accumulator claims.
    fn generate_inbox_proofs_for_claims(
        &self,
        target: AccountId,
        claims: &[AccumulatorClaim],
        at_leaf_count: u64,
    ) -> BlockAssemblyResult<Vec<RawMerkleProof>>;

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
            .map_err(|e| BlockAssemblyError::StateProvider(Box::new(e)))
            // keep current logic: stringified provider error
            .map(|opt| opt.map(Arc::new))
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
    ) -> BlockAssemblyResult<Vec<(OLTxId, OLTransaction)>> {
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
    ) -> BlockAssemblyResult<Vec<RawMerkleProof>> {
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

        // Return raw merkle proofs
        let inbox_proofs = merkle_proofs
            .into_iter()
            .map(|merkle_proof| merkle_proof.inner.clone())
            .collect();

        Ok(inbox_proofs)
    }

    fn generate_inbox_proofs_for_claims(
        &self,
        target: AccountId,
        claims: &[AccumulatorClaim],
        at_leaf_count: u64,
    ) -> BlockAssemblyResult<Vec<RawMerkleProof>> {
        if claims.is_empty() {
            return Ok(Vec::new());
        }

        let mmr_handle = self
            .storage
            .mmr_index()
            .as_ref()
            .get_handle(MmrId::SnarkMsgInbox(target));

        let indices_and_hashes: Vec<_> = claims
            .iter()
            .map(|claim| (claim.idx(), claim.entry_hash()))
            .collect();

        let merkle_proofs = mmr_handle
            .generate_proofs_for_indices(&indices_and_hashes, at_leaf_count)
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

        Ok(merkle_proofs
            .into_iter()
            .map(|merkle_proof| merkle_proof.inner.clone())
            .collect())
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

        // Claims already carry MMR leaf indices (not L1 heights), so use them
        // directly for proof generation.
        let indices_and_hashes: Vec<_> = l1_header_refs
            .iter()
            .map(|claim| (claim.idx(), claim.entry_hash()))
            .collect();

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

        let l1_header_proofs = merkle_proofs
            .into_iter()
            .map(|merkle_proof| merkle_proof.inner.clone())
            .collect();
        Ok(LedgerRefProofs::new(l1_header_proofs))
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::AccumulatorClaim;
    use strata_ol_state_support_types::MemoryStateBaseLayer;

    use super::*;
    use crate::test_utils::{
        TestAccount, TestStorageFixtureBuilder, create_test_context, create_test_message,
        test_account_id, test_hash,
    };

    // =========================================================================
    // L1 Header Proof Generation Tests
    // =========================================================================

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_header_proof_gen_success() {
        let account_id = test_account_id(1);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000))
            .with_l1_header_refs([1]);
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let claims = vec![
            fixture
                .l1_header_ref(1)
                .expect("claim for L1 height 1 should exist"),
        ];

        let ctx = create_test_context(fixture.storage().clone());
        let result = ctx
            .generate_l1_header_proofs(&claims, &MemoryStateBaseLayer::new(state.as_ref().clone()));

        assert!(result.is_ok(), "Should succeed with valid claim");
        let proofs = result.unwrap();
        assert_eq!(proofs.l1_headers_proofs().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_header_proof_gen_multiple_claims() {
        let account_id = test_account_id(1);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000))
            .with_l1_header_refs([1, 2, 3]);
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let claims = fixture
            .l1_header_refs()
            .iter()
            .map(|(_, claim)| claim.clone())
            .collect::<Vec<_>>();

        let ctx = create_test_context(fixture.storage().clone());
        let result = ctx
            .generate_l1_header_proofs(&claims, &MemoryStateBaseLayer::new(state.as_ref().clone()));

        assert!(result.is_ok(), "Should succeed with multiple valid claims");
        let proofs = result.unwrap();
        assert_eq!(proofs.l1_headers_proofs().len(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_header_proof_gen_hash_mismatch() {
        let account_id = test_account_id(1);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000))
            .with_l1_header_refs([1]);
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let seeded_claim = fixture
            .l1_header_ref(1)
            .expect("claim for L1 height 1 should exist");

        // Create claim with correct MMR index but wrong hash.
        let wrong_hash = test_hash(99);
        let claim = AccumulatorClaim::new(seeded_claim.idx(), wrong_hash);
        let expected_hash = seeded_claim.entry_hash();

        let ctx = create_test_context(fixture.storage().clone());

        let result = ctx.generate_l1_header_proofs(
            &[claim],
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
        );

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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_header_proof_gen_missing_index() {
        let account_id = test_account_id(1);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000))
            .with_l1_header_refs([1]);
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let seeded_claim = fixture
            .l1_header_ref(1)
            .expect("claim for L1 height 1 should exist");

        // Create claim with non-existent MMR index (999 doesn't exist, MMR has 1 entry)
        let nonexistent_idx = 999u64;
        let claim = AccumulatorClaim::new(nonexistent_idx, seeded_claim.entry_hash());

        let ctx = create_test_context(fixture.storage().clone());

        let result = ctx.generate_l1_header_proofs(
            &[claim],
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
        );

        assert!(result.is_err(), "Should fail with missing index");
        let err = result.unwrap_err();
        assert!(
            matches!(
                &err,
                BlockAssemblyError::Db(DbError::MmrIndexOutOfRange { requested, cur })
                    if *requested == nonexistent_idx && *cur == 1
            ),
            "Expected Db(MmrIndexOutOfRange) error, got: {:?}",
            err
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_header_claim_empty_mmr() {
        let account_id = test_account_id(1);
        let fixture_builder =
            TestStorageFixtureBuilder::new().with_account(TestAccount::new(account_id, 100_000));
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");

        // MMR index 0 is valid but the MMR is empty
        let claim = AccumulatorClaim::new(0, test_hash(42));
        let ctx = create_test_context(fixture.storage().clone());

        let result = ctx.generate_l1_header_proofs(
            &[claim],
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
        );

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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_header_proof_gen_empty_claims() {
        let account_id = test_account_id(1);
        let fixture_builder =
            TestStorageFixtureBuilder::new().with_account(TestAccount::new(account_id, 100_000));
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let ctx = create_test_context(fixture.storage().clone());

        let result =
            ctx.generate_l1_header_proofs(&[], &MemoryStateBaseLayer::new(state.as_ref().clone()));

        assert!(result.is_ok(), "Should succeed with empty claims");
        let proofs = result.unwrap();
        assert!(proofs.l1_headers_proofs().is_empty());
    }

    // =========================================================================
    // Inbox Proof Generation Tests
    // =========================================================================

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_proof_gen_success() {
        let account_id = test_account_id(1);
        let messages: Vec<_> = (1..=2)
            .map(|i| create_test_message(i, i as u32, 1000 * i as u64))
            .collect();
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000).with_inbox(messages.clone()));
        let (fixture, _parent_commitment) = fixture_builder.build_fixture().await;
        let ctx = create_test_context(fixture.storage().clone());
        let result = ctx.generate_inbox_proofs_at(account_id, &messages, 0, messages.len() as u64);

        assert!(
            result.is_ok(),
            "Should succeed with valid messages, got: {:?}",
            result.err()
        );
        let proofs = result.unwrap();
        assert_eq!(proofs.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_proof_gen_empty_messages() {
        let account_id = test_account_id(1);
        let fixture_builder =
            TestStorageFixtureBuilder::new().with_account(TestAccount::new(account_id, 100_000));
        let (fixture, _parent_commitment) = fixture_builder.build_fixture().await;
        let ctx = create_test_context(fixture.storage().clone());

        let result = ctx.generate_inbox_proofs_at(account_id, &[], 0, 0);

        assert!(result.is_ok(), "Should succeed with empty messages");
        let proofs = result.unwrap();
        assert!(proofs.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_proof_gen_with_offset() {
        let account_id = test_account_id(1);
        let all_messages: Vec<_> = (1..=4)
            .map(|i| create_test_message(i, i as u32, 1000 * i as u64))
            .collect();
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000).with_inbox(all_messages.clone()));
        let (fixture, _parent_commitment) = fixture_builder.build_fixture().await;
        let ctx = create_test_context(fixture.storage().clone());

        // Request proofs starting at index 2 for last 2 messages
        let messages_to_prove = &all_messages[2..];
        let result = ctx.generate_inbox_proofs_at(
            account_id,
            messages_to_prove,
            2,
            all_messages.len() as u64,
        );

        assert!(
            result.is_ok(),
            "Should succeed with offset, got: {:?}",
            result.err()
        );
        let proofs = result.unwrap();
        assert_eq!(proofs.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_proof_gen_missing_messages() {
        let account_id = test_account_id(1);
        let fixture_builder =
            TestStorageFixtureBuilder::new().with_account(TestAccount::new(account_id, 100_000));
        let (fixture, _parent_commitment) = fixture_builder.build_fixture().await;
        let ctx = create_test_context(fixture.storage().clone());
        let messages = vec![create_test_message(1, 1, 1000)];
        let result = ctx.generate_inbox_proofs_at(account_id, &messages, 0, 0);

        assert!(result.is_err(), "Should fail when MMR has no messages");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_claim_missing_index() {
        let account_id = test_account_id(1);
        let stored_message = create_test_message(1, 1, 1000);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000).with_inbox(vec![stored_message]));
        let (fixture, _parent_commitment) = fixture_builder.build_fixture().await;
        let ctx = create_test_context(fixture.storage().clone());
        let claimed_messages = vec![create_test_message(2, 2, 2000)];
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_claim_hash_mismatch() {
        let account_id = test_account_id(1);
        let stored_message = create_test_message(1, 1, 1000);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000).with_inbox(vec![stored_message]));
        let (fixture, _parent_commitment) = fixture_builder.build_fixture().await;
        let ctx = create_test_context(fixture.storage().clone());
        let claimed_messages = vec![create_test_message(2, 2, 2000)];
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
