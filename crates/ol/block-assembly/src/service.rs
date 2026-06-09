//! OL block assembly service implementation.

use std::{fmt::Display, marker::PhantomData};

use strata_identifiers::{Buf32, OLBlockId};
use strata_ledger_types::{IAccountStateMut, IStateAccessor, IStateAccessorMut};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, verify_sequencer_predicate_signature};
use strata_ol_state_provider::StateProvider;
use strata_predicate::PredicateKey;
use strata_service::{AsyncService, Response, Service};
use tracing::debug;

use crate::{
    BlockAssemblyStateAccess, EpochSealingPolicy, FullBlockTemplate, MempoolProvider,
    block_assembly::generate_block_template_inner,
    command::BlockasmCommand,
    error::BlockAssemblyError,
    state::{BlockTemplateStatus, BlockasmServiceState},
    types::{BlockCompletionData, BlockGenerationConfig},
};

/// OL block assembly service that processes commands.
#[derive(Debug)]
pub(crate) struct BlockasmService<M: MempoolProvider, E: EpochSealingPolicy, S> {
    _phantom: PhantomData<(M, E, S)>,
}

impl<M, E, S> Service for BlockasmService<M, E, S>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider + Send + Sync + 'static,
    S::Error: Display,
    S::State: BlockAssemblyStateAccess,
{
    type State = BlockasmServiceState<M, E, S>;
    type Msg = BlockasmCommand;
    type Status = BlockasmServiceStatus;

    fn get_status(_state: &Self::State) -> Self::Status {
        BlockasmServiceStatus
    }
}

impl<M, E, S> AsyncService for BlockasmService<M, E, S>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider + Send + Sync + 'static,
    S::Error: Display,
    S::State: BlockAssemblyStateAccess,
    <<S::State as IStateAccessorMut>::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut:
        Clone,
{
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        if !matches!(&input, BlockasmCommand::RecordPersistedBlock { .. }) {
            // Lazily clean up expired templates on every command except the post-persist
            // state update, which must be able to observe an already-persisted block.
            let expired_template_ids = state.state_mut().cleanup_expired_templates();
            for template_id in expired_template_ids {
                state
                    .epoch_da_tracker_mut()
                    .remove_accumulated_da(template_id);
            }
        }

        match input {
            BlockasmCommand::GenerateBlockTemplate { config, completion } => {
                let result = generate_block_template(state, config).await;
                _ = completion.send(result).await;
            }

            BlockasmCommand::GetBlockTemplate {
                parent_block_id,
                completion,
            } => {
                let result = get_block_template(state, parent_block_id);
                _ = completion.send(result).await;
            }

            BlockasmCommand::CompleteBlockTemplate {
                template_id,
                data,
                completion,
            } => {
                let result = complete_block_template(state, template_id, data);
                _ = completion.send(result).await;
            }

            BlockasmCommand::ReleaseCompletedTemplateStatus {
                parent_block_id,
                block,
                completion,
            } => {
                let released = state
                    .state_mut()
                    .release_completed_template_status(parent_block_id, block);
                _ = completion.send(released).await;
            }

            BlockasmCommand::RecordPersistedBlock {
                template_id,
                completion,
            } => {
                let result = record_persisted_block(state, template_id);
                _ = completion.send(result).await;
            }
        }

        Ok(Response::Continue)
    }
}

/// Generate a new block template.
async fn generate_block_template<
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider + Send + Sync + 'static,
>(
    state: &mut BlockasmServiceState<M, E, S>,
    config: BlockGenerationConfig,
) -> Result<FullBlockTemplate, BlockAssemblyError>
where
    S::Error: Display,
    S::State: BlockAssemblyStateAccess,
    // FIXME(STR-2778): This looks ugly, should we have Clone bound for the associated types?
    <<S::State as IStateAccessor>::AccountState as IAccountStateMut>::SnarkAccountStateMut: Clone,
    <<S::State as IStateAccessorMut>::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut:
        Clone,
{
    let parent_blkid = config.parent_block_id();
    state
        .state_mut()
        .prune_completed_template_statuses_for_parent(config.parent_block_commitment());

    // Check if we already have a pending template for this parent block ID
    if let Ok(template) = state
        .state_mut()
        .get_pending_block_template_by_parent(parent_blkid)
    {
        return Ok(template);
    }

    if let Some(BlockTemplateStatus::Completed { block }) = state
        .state_mut()
        .get_template_status_by_parent(parent_blkid)
    {
        return Err(BlockAssemblyError::TemplateAlreadyCompletedForParent {
            parent: parent_blkid,
            block,
        });
    }

    let parent_da = state
        .fetch_epoch_da_until_parent(config.parent_block_commitment())
        .await?;

    let result = generate_block_template_inner(
        state.context(),
        state.epoch_sealing_policy(),
        state.sequencer_config(),
        config,
        parent_da,
        *state.ol_params().bridge_params(),
    )
    .await?;

    let (full_template, failed_txs, accumulated_da) = result.into_parts();

    // Report failed transactions back to mempool.
    if !failed_txs.is_empty() {
        debug!(
            count = failed_txs.len(),
            "Reporting failed transactions to mempool"
        );
        MempoolProvider::report_invalid_transactions(state.context(), &failed_txs).await?;
    }

    let template_id = full_template.get_blockid();

    let evicted_template_ids = state
        .state_mut()
        .insert_template(template_id, full_template.clone())?;

    // Store accumulated DA for the new block, removing parent entry.
    state
        .epoch_da_tracker_mut()
        .set_accumulated_da_and_remove_parent_entry(template_id, parent_blkid, accumulated_da);

    for evicted_template_id in evicted_template_ids {
        state
            .epoch_da_tracker_mut()
            .remove_accumulated_da(evicted_template_id);
    }

    Ok(full_template)
}

/// Look up a pending block template by parent block ID.
fn get_block_template<M: MempoolProvider, E: EpochSealingPolicy, S>(
    state: &mut BlockasmServiceState<M, E, S>,
    parent_block_id: OLBlockId,
) -> Result<FullBlockTemplate, BlockAssemblyError> {
    state
        .state_mut()
        .get_pending_block_template_by_parent(parent_block_id)
}

/// Completes a cached template with a signature.
///
/// The signature is provided by the caller (sequencer) via `BlockCompletionData`. The flow is:
/// 1. Sequencer calls `GenerateBlockTemplate` to get a template with header hash
/// 2. Sequencer signs the header hash externally (e.g., via signing service)
/// 3. Sequencer calls `CompleteBlockTemplate` with the signature
/// 4. This function validates the signature before completing the cached template
///
/// The completed block is returned to the caller, who is responsible for durably storing it,
/// recording the persisted block for the template, and submitting it to the Fork Choice Manager
/// (FCM).
fn complete_block_template<M: MempoolProvider, E: EpochSealingPolicy, S>(
    state: &mut BlockasmServiceState<M, E, S>,
    template_id: OLBlockId,
    completion_data: BlockCompletionData,
) -> Result<OLBlock, BlockAssemblyError> {
    let template_ref = state.state_mut().get_pending_block_template(template_id)?;

    // Verify the signature before returning a completed block. Failed signatures keep the
    // cached template retryable.
    if !check_completion_data(
        state.sequencer_predicate(),
        template_ref.header(),
        &completion_data,
    ) {
        return Err(BlockAssemblyError::InvalidSignature(template_id));
    }

    Ok(template_ref.complete_block_template(completion_data))
}

/// Records that a block has been stored for a template and removes its accumulated DA entry.
fn record_persisted_block<M: MempoolProvider, E: EpochSealingPolicy, S>(
    state: &mut BlockasmServiceState<M, E, S>,
    template_id: OLBlockId,
) -> Result<(), BlockAssemblyError> {
    state.state_mut().record_persisted_block(template_id)?;
    state
        .epoch_da_tracker_mut()
        .remove_accumulated_da(template_id);
    Ok(())
}

/// Checks whether the sequencer signature matches the template header.
fn check_completion_data(
    sequencer_predicate: &PredicateKey,
    header: &OLBlockHeader,
    completion: &BlockCompletionData,
) -> bool {
    let msg: Buf32 = header.compute_blkid().into();

    verify_sequencer_predicate_signature(sequencer_predicate, &msg, completion.signature())
}

/// Service status for OL block assembly.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct BlockasmServiceStatus;

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Instant};

    use strata_config::BlockAssemblyConfig;
    use strata_crypto::sign_schnorr_sig;
    use strata_identifiers::{Buf32, Buf64, OLBlockCommitment};
    use strata_ol_chain_types_new::test_utils::{schnorr_predicate, test_schnorr_keypair};
    use strata_ol_mempool::{MempoolTxInvalidReason, OLMempoolError};
    use strata_ol_params::OLParams;
    use strata_ol_state_provider::OLStateManagerProviderImpl;
    use strata_predicate::PredicateKey;

    use super::*;
    use crate::{
        command::create_completion,
        da_tracker::AccumulatedDaData,
        epoch_sealing::FixedSlotSealing,
        state::BlockasmServiceState,
        test_utils::{
            MempoolSnarkTxBuilder, MockMempoolFailMode, MockMempoolProvider,
            TEST_BLOCK_TEMPLATE_TTL, TestAccount, TestEnv, TestStorageFixtureBuilder,
            create_test_template, create_test_template_with_parent, test_account_id,
        },
        types::BlockCompletionData,
    };

    type TestServiceState = BlockasmServiceState<
        Arc<MockMempoolProvider>,
        FixedSlotSealing,
        OLStateManagerProviderImpl,
    >;

    async fn build_service_state_with_accounts(
        use_schnorr_predicate: bool,
        accounts: impl IntoIterator<Item = TestAccount>,
    ) -> (
        TestServiceState,
        Arc<MockMempoolProvider>,
        TestEnv,
        Option<Buf32>,
    ) {
        let (sequencer_predicate, signing_key) = if use_schnorr_predicate {
            let (sk, pk) = test_schnorr_keypair();
            (schnorr_predicate(&pk), Some(sk))
        } else {
            (PredicateKey::always_accept(), None)
        };

        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_accounts(accounts)
            .build_fixture()
            .await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);
        let mempool = env.mempool_arc();

        let state = BlockasmServiceState::new(
            Arc::new(OLParams::default()),
            Arc::new(BlockAssemblyConfig::new(TEST_BLOCK_TEMPLATE_TTL)),
            env.sequencer_config().clone(),
            sequencer_predicate,
            env.ctx_arc(),
            env.epoch_sealing_policy().clone(),
        );

        (state, mempool, env, signing_key)
    }

    async fn build_service_state(
        use_schnorr_predicate: bool,
    ) -> (
        TestServiceState,
        Arc<MockMempoolProvider>,
        TestEnv,
        Option<Buf32>,
    ) {
        build_service_state_with_accounts(use_schnorr_predicate, Vec::<TestAccount>::new()).await
    }

    fn valid_completion_data(
        template: &FullBlockTemplate,
        signing_key: Buf32,
    ) -> BlockCompletionData {
        let msg: Buf32 = template.header().compute_blkid().into();
        let signature = sign_schnorr_sig(&msg, &signing_key);
        BlockCompletionData::from_signature(signature)
    }

    /// Verifies that `process_input` lazily cleans up expired templates
    /// before handling the incoming command.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_process_input_cleans_up_expired_templates() {
        let (mut state, _mempool, env, _sk) = build_service_state(false).await;

        // Insert a template and backdate it to simulate expiration.
        let template = create_test_template();
        let template_id = template.get_blockid();
        let parent = *template.header().parent_blkid();

        state
            .state_mut()
            .insert_template(template_id, template)
            .expect("test template insert should succeed");
        state
            .state_mut()
            .set_template_created_at_for_test(template_id, Instant::now() - TEST_BLOCK_TEMPLATE_TTL)
            .expect("template should be present before backdating");

        // Send any command — the lazy cleanup in process_input runs before handling it.
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let (completion, _rx) = create_completion();
        let cmd = BlockasmCommand::GenerateBlockTemplate { config, completion };
        BlockasmService::<_, _, _>::process_input(&mut state, cmd)
            .await
            .unwrap();

        // Verify expired template was removed from both maps.
        assert!(matches!(
            state
                .state_mut()
                .get_pending_block_template(template_id),
            Err(BlockAssemblyError::UnknownTemplateId(id)) if id == template_id
        ));
        assert!(matches!(
            state
                .state_mut()
                .get_pending_block_template_by_parent(parent),
            Err(BlockAssemblyError::NoPendingTemplateForParent(p)) if p == parent
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_process_input_cleanup_evicts_expired_da_tracker_entry() {
        let (mut state, _mempool, env, _sk) = build_service_state(false).await;
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let template = generate_block_template(&mut state, config)
            .await
            .expect("generation should succeed");
        let template_id = template.get_blockid();
        let parent = *template.header().parent_blkid();
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(template_id)
                .is_some(),
            "generated template should have tracked DA entry"
        );

        state
            .state_mut()
            .set_template_created_at_for_test(template_id, Instant::now() - TEST_BLOCK_TEMPLATE_TTL)
            .expect("template should be present before backdating");

        let (completion, rx) = create_completion();
        let cmd = BlockasmCommand::GetBlockTemplate {
            parent_block_id: parent,
            completion,
        };
        BlockasmService::<_, _, _>::process_input(&mut state, cmd)
            .await
            .expect("process_input should succeed");
        let lookup_result = rx.await.expect("lookup completion should be delivered");
        assert!(
            matches!(
                lookup_result,
                Err(BlockAssemblyError::NoPendingTemplateForParent(p)) if p == parent
            ),
            "lookup should fail after expired template cleanup"
        );
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(template_id)
                .is_none(),
            "expired template cleanup should evict DA tracker entry"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_generate_reuses_cached_template() {
        let (mut state, mempool, env, _sk) = build_service_state(false).await;
        let config = BlockGenerationConfig::new(env.parent_commitment());

        let first = generate_block_template(&mut state, config.clone())
            .await
            .expect("first generation should succeed");
        let template_id = first.get_blockid();
        let tracked_da = state
            .epoch_da_tracker()
            .get_accumulated_da(template_id)
            .expect("first generation should store accumulated DA for template id");
        assert_eq!(
            tracked_da.logs().len(),
            0,
            "empty mempool generation should store zero DA logs"
        );

        // If generation ran again, this would fail. Cached reuse must short-circuit before mempool
        // fetch.
        mempool.set_fail_mode(MockMempoolFailMode::GetTransactions);
        let second = generate_block_template(&mut state, config)
            .await
            .expect("second generation should return cached template");
        assert_eq!(
            second.get_blockid(),
            template_id,
            "same parent should return exact cached template id"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_new_template_for_same_parent_evicts_expired_template_da_entry() {
        let sender = test_account_id(91);
        let (mut state, mempool, env, _sk) =
            build_service_state_with_accounts(false, [TestAccount::new(sender, 10_000)]).await;
        let config = BlockGenerationConfig::new(env.parent_commitment());

        let first = generate_block_template(&mut state, config.clone())
            .await
            .expect("first generation should succeed");
        let first_template_id = first.get_blockid();
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(first_template_id)
                .is_some(),
            "first generation should store accumulated DA entry"
        );
        state
            .state_mut()
            .set_template_created_at_for_test(
                first_template_id,
                Instant::now() - TEST_BLOCK_TEMPLATE_TTL,
            )
            .expect("first template should be present before backdating");

        // Make regenerated template content differ so block ID replacement path is deterministic.
        let tx = MempoolSnarkTxBuilder::new(sender).with_seq_no(0).build();
        mempool.add_transaction(tx.compute_txid(), tx);

        let second = generate_block_template(&mut state, config)
            .await
            .expect("regeneration after expiry should succeed");
        let second_template_id = second.get_blockid();
        assert_ne!(
            second_template_id, first_template_id,
            "regeneration should produce a distinct template id"
        );
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(first_template_id)
                .is_none(),
            "same-parent replacement should evict old DA tracker entry"
        );
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(second_template_id)
                .is_some(),
            "new template should retain DA tracker entry"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_insert_cleans_expired_da_entry_for_different_parent() {
        let (mut state, _mempool, env, _sk) = build_service_state(false).await;

        let stale_parent = OLBlockId::from(Buf32::from([0xAB; 32]));
        let stale_template = create_test_template_with_parent(stale_parent);
        let stale_template_id = stale_template.get_blockid();
        state
            .state_mut()
            .insert_template(stale_template_id, stale_template)
            .expect("stale test template insert should succeed");
        state
            .epoch_da_tracker_mut()
            .set_accumulated_da(stale_template_id, AccumulatedDaData::new_empty());
        state
            .state_mut()
            .set_template_created_at_for_test(
                stale_template_id,
                Instant::now() - TEST_BLOCK_TEMPLATE_TTL,
            )
            .expect("stale template should be present before backdating");

        let config = BlockGenerationConfig::new(env.parent_commitment());
        let _new_template = generate_block_template(&mut state, config)
            .await
            .expect("generation should succeed while cleaning unrelated stale entry");

        assert!(
            matches!(
                state.state_mut().get_pending_block_template(stale_template_id),
                Err(BlockAssemblyError::UnknownTemplateId(id)) if id == stale_template_id
            ),
            "unrelated expired template should be removed during insert-time cleanup"
        );
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(stale_template_id)
                .is_none(),
            "insert-time cleanup should evict stale DA tracker entry"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cached_template_does_not_report() {
        let (mut state, mempool, env, _sk) = build_service_state(false).await;
        let missing_account = test_account_id(77);
        let invalid_tx = MempoolSnarkTxBuilder::new(missing_account)
            .with_seq_no(0)
            .build();
        let invalid_txid = invalid_tx.compute_txid();
        mempool.add_transaction(invalid_txid, invalid_tx);

        let config = BlockGenerationConfig::new(env.parent_commitment());
        let first = generate_block_template(&mut state, config.clone())
            .await
            .expect("first generation should succeed despite invalid tx");
        assert_eq!(
            mempool.report_call_count(),
            1,
            "first generation should report failed txs exactly once"
        );
        assert_eq!(
            mempool.last_reported_invalid_txs(),
            vec![(invalid_txid, MempoolTxInvalidReason::Invalid)],
            "first generation should report missing-account tx as Invalid"
        );

        // Reuse path must not call report_invalid_transactions again.
        mempool.set_fail_mode(MockMempoolFailMode::ReportInvalidTransactions);
        let second = generate_block_template(&mut state, config)
            .await
            .expect("cached template reuse should not hit report path");
        assert_eq!(second.get_blockid(), first.get_blockid());
        assert_eq!(
            mempool.report_call_count(),
            1,
            "cached template reuse should not add extra report calls"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_generate_propagates_report_invalid_transactions_failure() {
        let (mut state, mempool, env, _sk) = build_service_state(false).await;
        let missing_account = test_account_id(78);
        let invalid_tx = MempoolSnarkTxBuilder::new(missing_account)
            .with_seq_no(0)
            .build();
        let invalid_txid = invalid_tx.compute_txid();
        mempool.add_transaction(invalid_txid, invalid_tx);
        mempool.set_fail_mode(MockMempoolFailMode::ReportInvalidTransactions);

        let config = BlockGenerationConfig::new(env.parent_commitment());
        let err = generate_block_template(&mut state, config)
            .await
            .expect_err("report_invalid_transactions failure should fail generation");
        assert!(
            matches!(
                err,
                BlockAssemblyError::Mempool(OLMempoolError::ServiceClosed(_))
            ),
            "expected mempool service-closed error, got: {err:?}"
        );
        assert_eq!(
            mempool.report_call_count(),
            1,
            "failed tx report should be attempted exactly once at service layer"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_always_accept_completes_with_garbage_signature() {
        let (mut state, _mempool, env, _sk) = build_service_state(false).await;
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let template = generate_block_template(&mut state, config)
            .await
            .expect("generation should succeed");
        let template_id = template.get_blockid();

        let completion_data = BlockCompletionData::from_signature(Buf64::zero());
        let completed = complete_block_template(&mut state, template_id, completion_data)
            .expect("AlwaysAccept should allow the always-present signature bytes");
        assert_eq!(
            completed.header().compute_blkid(),
            template_id,
            "completed block id should match template id"
        );

        let cached_template = state
            .state_mut()
            .get_pending_block_template(template_id)
            .expect("template should remain cached until storage succeeds");
        assert_eq!(cached_template.get_blockid(), template_id);

        record_persisted_block(&mut state, template_id)
            .expect("persisted block should be recorded for template after storage succeeds");
        assert!(
            matches!(
                state.state_mut().get_pending_block_template(template_id),
                Err(BlockAssemblyError::UnknownTemplateId(id)) if id == template_id
            ),
            "template should be removed after persisted block is recorded"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_invalid_signature_keeps_template_cached() {
        let (mut state, _mempool, env, _sk) = build_service_state(true).await;
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let template = generate_block_template(&mut state, config)
            .await
            .expect("generation should succeed");
        let template_id = template.get_blockid();

        let bad_completion = BlockCompletionData::from_signature(Buf64::zero());
        let err = complete_block_template(&mut state, template_id, bad_completion)
            .expect_err("invalid signature should fail completion");
        assert!(
            matches!(err, BlockAssemblyError::InvalidSignature(id) if id == template_id),
            "expected InvalidSignature({template_id}), got {err:?}"
        );
        let cached_template = state
            .state_mut()
            .get_pending_block_template(template_id)
            .expect("template should remain cached after invalid signature");
        assert_eq!(
            cached_template.get_blockid(),
            template_id,
            "cached template id should remain unchanged after invalid signature"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_second_template_completion_rejected() {
        let (mut state, _mempool, env, sk) = build_service_state(true).await;
        let signing_key = sk.expect("schnorr signing key should be present");
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let template = generate_block_template(&mut state, config)
            .await
            .expect("generation should succeed");
        let template_id = template.get_blockid();

        let completion_data = valid_completion_data(&template, signing_key);
        let completed = complete_block_template(&mut state, template_id, completion_data.clone())
            .expect("valid signature should complete template");
        assert_eq!(
            completed.header().compute_blkid(),
            template_id,
            "completed block id should match template id"
        );

        let cached_template = state
            .state_mut()
            .get_pending_block_template(template_id)
            .expect("template should remain cached until storage succeeds");
        assert_eq!(cached_template.get_blockid(), template_id);
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(template_id)
                .is_some(),
            "completion should not evict DA tracker entry before storage succeeds"
        );

        record_persisted_block(&mut state, template_id)
            .expect("persisted block should be recorded for template after storage succeeds");
        assert!(
            matches!(
                state.state_mut().get_pending_block_template(template_id),
                Err(BlockAssemblyError::UnknownTemplateId(id)) if id == template_id
            ),
            "template should be removed after successful completion"
        );
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(template_id)
                .is_none(),
            "successful completion should evict DA tracker entry"
        );

        let err = complete_block_template(&mut state, template_id, completion_data)
            .expect_err("second completion should fail");
        assert!(
            matches!(err, BlockAssemblyError::UnknownTemplateId(id) if id == template_id),
            "second completion should return UnknownTemplateId, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_record_persisted_block_skips_ttl_cleanup() {
        let (mut state, _mempool, env, sk) = build_service_state(true).await;
        let signing_key = sk.expect("schnorr signing key should be present");
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let template = generate_block_template(&mut state, config)
            .await
            .expect("generation should succeed");
        let template_id = template.get_blockid();

        let completion_data = valid_completion_data(&template, signing_key);
        complete_block_template(&mut state, template_id, completion_data)
            .expect("valid signature should complete template");

        state
            .state_mut()
            .set_template_created_at_for_test(template_id, Instant::now() - TEST_BLOCK_TEMPLATE_TTL)
            .expect("template should be present before backdating");

        let (completion, rx) = create_completion();
        let cmd = BlockasmCommand::RecordPersistedBlock {
            template_id,
            completion,
        };
        BlockasmService::<_, _, _>::process_input(&mut state, cmd)
            .await
            .unwrap();
        rx.await
            .expect("completion sender should respond")
            .expect("recording persisted block should ignore template TTL after storage succeeds");

        assert!(
            matches!(
                state.state_mut().get_pending_block_template(template_id),
                Err(BlockAssemblyError::UnknownTemplateId(id)) if id == template_id
            ),
            "template should be removed after persisted block is recorded"
        );
        assert!(
            state
                .epoch_da_tracker()
                .get_accumulated_da(template_id)
                .is_none(),
            "recording persisted block should evict DA tracker entry"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_generation_after_completion_returns_completed_parent_error() {
        let (mut state, mempool, env, sk) = build_service_state(true).await;
        let signing_key = sk.expect("schnorr signing key should be present");
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let parent = config.parent_block_id();
        let template = generate_block_template(&mut state, config.clone())
            .await
            .expect("generation should succeed");
        let template_id = template.get_blockid();

        let completion_data = valid_completion_data(&template, signing_key);
        let completed = complete_block_template(&mut state, template_id, completion_data)
            .expect("valid signature should complete template");
        let completed_commitment = OLBlockCommitment::new(
            completed.header().slot(),
            completed.header().compute_blkid(),
        );
        record_persisted_block(&mut state, template_id)
            .expect("persisted block should be recorded for template after storage succeeds");

        // If generation tries to rebuild, this fail mode makes the test fail.
        mempool.set_fail_mode(MockMempoolFailMode::GetTransactions);
        let err = generate_block_template(&mut state, config)
            .await
            .expect_err("completed parent should not generate a fresh template");

        assert!(
            matches!(
                err,
                BlockAssemblyError::TemplateAlreadyCompletedForParent {
                    parent: err_parent,
                    block: err_block,
                } if err_parent == parent && err_block == completed_commitment
            ),
            "expected TemplateAlreadyCompletedForParent, got: {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_release_completed_template_status_command_releases_exact_match() {
        let (mut state, _mempool, env, sk) = build_service_state(true).await;
        let signing_key = sk.expect("schnorr signing key should be present");
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let parent = config.parent_block_id();
        let template = generate_block_template(&mut state, config)
            .await
            .expect("generation should succeed");
        let template_id = template.get_blockid();

        let completion_data = valid_completion_data(&template, signing_key);
        let completed = complete_block_template(&mut state, template_id, completion_data)
            .expect("valid signature should complete template");
        record_persisted_block(&mut state, template_id)
            .expect("completed template should be recorded as persisted");
        let completed_commitment = OLBlockCommitment::new(
            completed.header().slot(),
            completed.header().compute_blkid(),
        );

        let (completion, rx) = create_completion();
        let cmd = BlockasmCommand::ReleaseCompletedTemplateStatus {
            parent_block_id: parent,
            block: completed_commitment,
            completion,
        };
        BlockasmService::<_, _, _>::process_input(&mut state, cmd)
            .await
            .expect("process_input should succeed");

        assert!(
            rx.await.expect("release completion should be delivered"),
            "completed status should be released when parent and block match"
        );
        assert!(
            state
                .state_mut()
                .get_template_status_by_parent(parent)
                .is_none(),
            "released completed status should be removed"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_generation_after_completed_status_release_rebuilds_template() {
        let sender = test_account_id(92);
        let (mut state, mempool, env, sk) =
            build_service_state_with_accounts(true, [TestAccount::new(sender, 10_000)]).await;
        let signing_key = sk.expect("schnorr signing key should be present");
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let parent = config.parent_block_id();
        let first = generate_block_template(&mut state, config.clone())
            .await
            .expect("first generation should succeed");
        let first_template_id = first.get_blockid();

        let completion_data = valid_completion_data(&first, signing_key);
        let completed = complete_block_template(&mut state, first_template_id, completion_data)
            .expect("valid signature should complete template");
        record_persisted_block(&mut state, first_template_id)
            .expect("completed template should be recorded as persisted");
        let completed_commitment = OLBlockCommitment::new(
            completed.header().slot(),
            completed.header().compute_blkid(),
        );
        assert_eq!(
            state.state_mut().get_template_status_by_parent(parent),
            Some(BlockTemplateStatus::Completed {
                block: completed_commitment
            }),
            "completed template should leave a completed parent status"
        );

        assert!(
            state
                .state_mut()
                .release_completed_template_status(parent, completed_commitment),
            "matching completed status should be released"
        );
        assert!(
            state
                .state_mut()
                .get_template_status_by_parent(parent)
                .is_none(),
            "released parent should not retain a completed status"
        );

        let tx = MempoolSnarkTxBuilder::new(sender).with_seq_no(0).build();
        mempool.add_transaction(tx.compute_txid(), tx);

        let second = generate_block_template(&mut state, config)
            .await
            .expect("generation should rebuild after completed status release");
        assert_eq!(
            second.header().slot(),
            completed_commitment.slot(),
            "replacement template should target the same child slot"
        );
        assert_ne!(
            second.get_blockid(),
            first_template_id,
            "released parent should rebuild a distinct template"
        );
        assert_eq!(
            state.state_mut().get_template_status_by_parent(parent),
            Some(BlockTemplateStatus::Pending {
                template_id: second.get_blockid()
            }),
            "released parent should track the replacement template as pending"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_expired_template_completion_rejected() {
        let (mut state, _mempool, env, _sk) = build_service_state(false).await;
        let config = BlockGenerationConfig::new(env.parent_commitment());
        let template = generate_block_template(&mut state, config)
            .await
            .expect("generation should succeed");
        let template_id = template.get_blockid();

        state
            .state_mut()
            .set_template_created_at_for_test(template_id, Instant::now() - TEST_BLOCK_TEMPLATE_TTL)
            .expect("template should be present before backdating");

        let err = complete_block_template(
            &mut state,
            template_id,
            BlockCompletionData::from_signature(Buf64::zero()),
        )
        .expect_err("expired template completion should fail");
        assert!(
            matches!(err, BlockAssemblyError::UnknownTemplateId(id) if id == template_id),
            "expired completion should return UnknownTemplateId, got {err:?}"
        );
    }
}
