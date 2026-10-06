//! Checks that loaded artifacts match every active and pending checkpoint VK at startup.
//!
//! Reads locally canonical ASM state without waiting for L1 or ASM to catch up.

use strata_asm_common::{AnchorState, AsmError, SectionStateExt, Subprotocol};
use strata_asm_proto_checkpoint::{CheckpointState, CheckpointSubprotocol};
use strata_db_types::DbError;
use strata_identifiers::L1BlockCommitment;
use strata_predicate::PredicateTypeId;
use strata_storage::NodeStorage;

use crate::LoadedCheckpointPredicates;
use crate::artifact_checks::{
    CheckpointPredicateStatus, MissingCheckpointArtifacts, check_predicate_artifact,
    ensure_artifacts_available,
};

/// Reports a failure to read or decode ASM checkpoint state.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointStateReadError {
    /// The canonical ASM snapshot could not be read.
    #[error("failed to read canonical ASM checkpoint state")]
    CanonicalAsm(#[source] DbError),
    /// The persisted canonical snapshot has no checkpoint section.
    #[error("canonical ASM state is missing the checkpoint section")]
    MissingCheckpointSection,
    /// The checkpoint section could not be decoded.
    #[error("failed to decode the ASM checkpoint section")]
    DecodeCheckpoint(#[source] AsmError),
}

/// Reports checkpoint state or prover configuration that prevents startup.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointArtifactCheckError {
    /// No locally canonical ASM state is available for the startup check.
    #[error("canonical ASM checkpoint state is unavailable")]
    StateUnavailable,
    /// Required protocol state could not be read or decoded.
    #[error(transparent)]
    Read(#[from] CheckpointStateReadError),
    /// An active or pending checkpoint VK has no matching loaded artifact.
    #[error(transparent)]
    MissingArtifacts(#[from] MissingCheckpointArtifacts),
    /// Empty proofs cannot satisfy an active or pending checkpoint predicate.
    #[error(
        "active or pending checkpoint predicate requires a prover, but no prover is enabled; \
         configure [prover] and build strata with the prover feature"
    )]
    ProverRequired,
}

/// Requires artifacts for all active and pending checkpoint VKs and returns the checked L1 block.
///
/// Uses the canonical stored ASM anchor, including a genesis anchor without logs. Before
/// the L1 reader has created its canonical index, it accepts only the persisted configured
/// genesis anchor, with no later anchor. The caller must start the ASM worker first: its
/// initialization validates this configured genesis commitment against Bitcoin.
///
/// Each task selects its artifact from its epoch's OL state, independently of ASM's
/// current active VK. This check only establishes artifact availability at startup.
/// This function performs blocking storage reads and must not run on an async executor.
pub fn check_startup_artifacts_blocking(
    storage: &NodeStorage,
    predicates: &LoadedCheckpointPredicates,
    genesis_block: L1BlockCommitment,
) -> Result<L1BlockCommitment, CheckpointArtifactCheckError> {
    let (block, checkpoint) = read_startup_checkpoint_blocking(storage, genesis_block)?;
    check_checkpoint_artifacts(predicates, &checkpoint)?;
    Ok(block)
}

/// Allows empty checkpoint proofs only when every active and pending predicate is AlwaysAccept.
///
/// The caller must use this check for a sequencer without an enabled prover, after ASM
/// initialization and before checkpoint production. Uses the same local ASM snapshot as
/// [`check_startup_artifacts_blocking`], without waiting for L1 or ASM to catch up.
/// This function performs blocking storage reads and must not run on an async executor.
pub fn check_startup_without_prover_blocking(
    storage: &NodeStorage,
    genesis_block: L1BlockCommitment,
) -> Result<L1BlockCommitment, CheckpointArtifactCheckError> {
    let (block, checkpoint) = read_startup_checkpoint_blocking(storage, genesis_block)?;
    let always_accept_id = PredicateTypeId::AlwaysAccept.as_u8();
    if checkpoint.checkpoint_predicate().id() != always_accept_id
        || checkpoint
            .pending_transitions()
            .iter()
            .any(|transition| transition.predicate().id() != always_accept_id)
    {
        return Err(CheckpointArtifactCheckError::ProverRequired);
    }
    Ok(block)
}

fn read_startup_checkpoint_blocking(
    storage: &NodeStorage,
    genesis_block: L1BlockCommitment,
) -> Result<(L1BlockCommitment, CheckpointState), CheckpointArtifactCheckError> {
    match read_canonical_checkpoint_blocking(storage)? {
        Some(snapshot) => Ok(snapshot),
        None => read_genesis_checkpoint_blocking(storage, genesis_block)?
            .ok_or(CheckpointArtifactCheckError::StateUnavailable),
    }
}

/// Reads persisted genesis while the canonical L1 index is still absent.
///
/// Genesis must be the latest stored anchor and match the configured commitment.
/// Otherwise, retries the canonical reader in case the L1 reader has advanced.
fn read_genesis_checkpoint_blocking(
    storage: &NodeStorage,
    genesis_block: L1BlockCommitment,
) -> Result<Option<(L1BlockCommitment, CheckpointState)>, CheckpointStateReadError> {
    let latest = storage
        .asm()
        .fetch_most_recent_anchor_state_blocking()
        .map_err(CheckpointStateReadError::CanonicalAsm)?;
    if let Some((block, anchor)) = latest
        && block == genesis_block
        && storage
            .l1()
            .get_canonical_chain_tip()
            .map_err(CheckpointStateReadError::CanonicalAsm)?
            .is_none()
    {
        return Ok(Some((block, decode_checkpoint_section(&anchor)?)));
    }

    read_canonical_checkpoint_blocking(storage)
}

/// Reads local canonical ASM checkpoint state without waiting for catch-up.
fn read_canonical_checkpoint_blocking(
    storage: &NodeStorage,
) -> Result<Option<(L1BlockCommitment, CheckpointState)>, CheckpointStateReadError> {
    let Some((block, anchor)) = storage
        .fetch_canonical_asm_anchor_state_blocking()
        .map_err(CheckpointStateReadError::CanonicalAsm)?
    else {
        return Ok(None);
    };
    Ok(Some((block, decode_checkpoint_section(&anchor)?)))
}

fn check_checkpoint_artifacts(
    predicates: &LoadedCheckpointPredicates,
    checkpoint: &CheckpointState,
) -> Result<(), MissingCheckpointArtifacts> {
    let mut missing_artifacts = Vec::new();
    missing_artifacts.extend(check_predicate_artifact(
        predicates,
        CheckpointPredicateStatus::Active,
        checkpoint.checkpoint_predicate(),
    ));
    for transition in checkpoint.pending_transitions() {
        missing_artifacts.extend(check_predicate_artifact(
            predicates,
            CheckpointPredicateStatus::Pending {
                boundary: transition.boundary(),
            },
            transition.predicate(),
        ));
    }

    ensure_artifacts_available(&missing_artifacts)
}

/// Decodes the checkpoint subprotocol's state from an ASM anchor.
fn decode_checkpoint_section(
    anchor: &AnchorState,
) -> Result<CheckpointState, CheckpointStateReadError> {
    anchor
        .find_section(CheckpointSubprotocol::ID)
        .ok_or(CheckpointStateReadError::MissingCheckpointSection)?
        .try_to_state::<CheckpointSubprotocol>()
        .map_err(CheckpointStateReadError::DecodeCheckpoint)
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use strata_asm_checkpoint_types::{CheckpointInitConfig, PendingPredicateTransition};
    use strata_asm_common::SectionState;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_tests::asm_tests::make_test_asm_state;
    use strata_db_types::asm::AsmExecOutput;
    use strata_identifiers::{Buf32, L1BlockId, OLBlockId};
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_state_types::OLSpecId;
    use strata_predicate::PredicateKey;
    use strata_storage::{create_node_storage, test_runtime_handle};

    use super::*;
    use crate::{CheckpointArtifact, CheckpointArtifactRegistry, native_checkpoint_registry};

    fn storage() -> NodeStorage {
        create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap()
    }

    fn loaded_predicates() -> LoadedCheckpointPredicates {
        native_checkpoint_registry(OLRuntimeParams::test_default()).to_predicates()
    }

    fn checkpoint(predicates: &LoadedCheckpointPredicates) -> CheckpointState {
        CheckpointState::init(CheckpointInitConfig {
            sequencer_key: Buf32::zero(),
            checkpoint_predicate: predicates.predicate(OLSpecId::V1).unwrap().clone(),
            genesis_l1_height: 0,
            genesis_ol_blkid: OLBlockId::from(Buf32::from([1; 32])),
        })
    }

    fn anchor_state(checkpoint: &CheckpointState) -> AnchorState {
        let mut anchor = make_test_asm_state().state().clone();
        // The check must work without unrelated sections, including administration state.
        anchor.sections =
            vec![SectionState::from_state::<CheckpointSubprotocol>(checkpoint).unwrap()]
                .try_into()
                .unwrap();
        anchor
    }

    fn extend_snapshot(
        storage: &NodeStorage,
        height: u32,
        checkpoint: &CheckpointState,
    ) -> L1BlockCommitment {
        let block =
            L1BlockCommitment::new(height, L1BlockId::from(Buf32::from([height as u8; 32])));
        storage
            .l1()
            .extend_canonical_chain(block.blkid(), height)
            .unwrap();
        let anchor = anchor_state(checkpoint);
        storage
            .asm()
            .put_state_blocking(block, AsmExecOutput::new(anchor, Vec::new()))
            .unwrap();
        block
    }

    #[test]
    fn startup_without_prover_accepts_always_accept_genesis_and_pending_predicates() {
        let storage = storage();
        let mut checkpoint = checkpoint(&loaded_predicates());
        checkpoint.checkpoint_predicate = PredicateKey::always_accept();
        checkpoint.queue_predicate_transition(PendingPredicateTransition::new(
            PredicateKey::always_accept(),
            10,
        ));
        let genesis = L1BlockCommitment::new(1, L1BlockId::from(Buf32::from([1; 32])));
        storage
            .asm()
            .put_anchor_state_blocking(genesis, anchor_state(&checkpoint))
            .unwrap();
        assert_eq!(storage.l1().get_canonical_chain_tip().unwrap(), None);
        assert_eq!(
            check_startup_without_prover_blocking(&storage, genesis).unwrap(),
            genesis,
        );
    }

    #[test]
    fn startup_without_prover_accepts_always_accept_with_nonempty_conditions() {
        let storage = storage();
        let mut checkpoint = checkpoint(&loaded_predicates());
        checkpoint.checkpoint_predicate =
            PredicateKey::try_new(PredicateTypeId::AlwaysAccept, vec![1]).unwrap();
        checkpoint.queue_predicate_transition(PendingPredicateTransition::new(
            PredicateKey::try_new(PredicateTypeId::AlwaysAccept, vec![2]).unwrap(),
            10,
        ));
        let block = extend_snapshot(&storage, 1, &checkpoint);
        assert_eq!(
            check_startup_without_prover_blocking(&storage, block).unwrap(),
            block,
        );
    }

    #[test]
    fn startup_without_prover_checks_current_active_and_pending_predicates() {
        let storage = storage();
        let mut checkpoint = checkpoint(&loaded_predicates());
        let proof_predicate = checkpoint.checkpoint_predicate().clone();
        let genesis = extend_snapshot(&storage, 1, &checkpoint);
        assert!(matches!(
            check_startup_without_prover_blocking(&storage, genesis),
            Err(CheckpointArtifactCheckError::ProverRequired)
        ));

        // A retired genesis key must not restrict the current AlwaysAccept state.
        checkpoint.checkpoint_predicate = PredicateKey::always_accept();
        let current = extend_snapshot(&storage, 2, &checkpoint);
        assert_eq!(
            check_startup_without_prover_blocking(&storage, genesis).unwrap(),
            current,
        );

        // A future key already in the enacted queue must be supported at startup.
        checkpoint.queue_predicate_transition(PendingPredicateTransition::new(proof_predicate, 10));
        extend_snapshot(&storage, 3, &checkpoint);
        assert!(matches!(
            check_startup_without_prover_blocking(&storage, genesis),
            Err(CheckpointArtifactCheckError::ProverRequired)
        ));

        storage.l1().revert_canonical_chain(2).unwrap();
        assert_eq!(
            check_startup_without_prover_blocking(&storage, genesis).unwrap(),
            current,
        );
    }

    #[test]
    fn startup_checks_vks_without_reading_ol_state() {
        let storage = storage();
        let predicates = loaded_predicates();
        let checkpoint = checkpoint(&predicates);
        assert!(
            storage
                .ol_state()
                .get_toplevel_ol_state_blocking(*checkpoint.verified_tip().l2_commitment())
                .unwrap()
                .is_none()
        );
        let block = extend_snapshot(&storage, 1, &checkpoint);
        let checked_block = check_startup_artifacts_blocking(&storage, &predicates, block).unwrap();
        assert_eq!(checked_block, block);
    }

    #[test]
    fn startup_ignores_orphaned_predicates_after_l1_rollback() {
        let storage = storage();
        let predicates = loaded_predicates();
        let mut checkpoint = checkpoint(&predicates);
        let original = extend_snapshot(&storage, 1, &checkpoint);
        checkpoint.queue_predicate_transition(PendingPredicateTransition::new(
            PredicateKey::always_accept(),
            10,
        ));
        extend_snapshot(&storage, 2, &checkpoint);
        assert!(matches!(
            check_startup_artifacts_blocking(&storage, &predicates, original),
            Err(CheckpointArtifactCheckError::MissingArtifacts(_))
        ));

        storage.l1().revert_canonical_chain(1).unwrap();
        let restored = check_startup_artifacts_blocking(&storage, &predicates, original).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn startup_rejects_missing_pending_artifacts_after_enactment() {
        let storage = storage();
        let predicates = loaded_predicates();
        let mut checkpoint = checkpoint(&predicates);
        let genesis = extend_snapshot(&storage, 1, &checkpoint);
        let initial = check_startup_artifacts_blocking(&storage, &predicates, genesis).unwrap();
        assert_eq!(initial, genesis);

        checkpoint.queue_predicate_transition(PendingPredicateTransition::new(
            PredicateKey::always_accept(),
            10,
        ));
        extend_snapshot(&storage, 2, &checkpoint);
        let error = check_startup_artifacts_blocking(&storage, &predicates, genesis).unwrap_err();
        let CheckpointArtifactCheckError::MissingArtifacts(error) = error else {
            panic!("expected missing pending artifact");
        };
        let [missing] = error.missing_artifacts() else {
            panic!("expected one missing artifact");
        };
        assert_eq!(
            missing.status(),
            CheckpointPredicateStatus::Pending { boundary: 10 }
        );
        assert_eq!(missing.predicate(), &PredicateKey::always_accept());
    }

    #[test]
    fn startup_rejects_missing_active_predicate() {
        let storage = storage();
        let predicates = loaded_predicates();
        let mut checkpoint = checkpoint(&predicates);
        checkpoint.checkpoint_predicate = PredicateKey::always_accept();
        let genesis = extend_snapshot(&storage, 1, &checkpoint);
        let error = check_startup_artifacts_blocking(&storage, &predicates, genesis).unwrap_err();
        let CheckpointArtifactCheckError::MissingArtifacts(error) = error else {
            panic!("expected missing active artifact");
        };
        let [missing] = error.missing_artifacts() else {
            panic!("expected one missing artifact");
        };
        assert_eq!(missing.status(), CheckpointPredicateStatus::Active);
        assert_eq!(missing.predicate(), &PredicateKey::always_accept());
    }

    #[test]
    fn startup_validates_canonical_genesis_anchor_without_logs() {
        let storage = storage();
        let predicates = loaded_predicates();
        let checkpoint = checkpoint(&predicates);
        let genesis = L1BlockCommitment::new(1, L1BlockId::from(Buf32::from([1; 32])));
        storage
            .l1()
            .extend_canonical_chain(genesis.blkid(), genesis.height())
            .unwrap();
        storage
            .asm()
            .put_anchor_state_blocking(genesis, anchor_state(&checkpoint))
            .unwrap();
        assert_eq!(storage.fetch_canonical_asm_state_blocking().unwrap(), None);
        let checked_block =
            check_startup_artifacts_blocking(&storage, &predicates, genesis).unwrap();
        assert_eq!(checked_block, genesis);
    }

    #[test]
    fn startup_before_l1_index_requires_exact_persisted_genesis_and_artifacts() {
        let storage = storage();
        let predicates = loaded_predicates();
        let checkpoint = checkpoint(&predicates);
        let genesis = L1BlockCommitment::new(1, L1BlockId::from(Buf32::from([1; 32])));
        assert!(matches!(
            check_startup_artifacts_blocking(&storage, &predicates, genesis),
            Err(CheckpointArtifactCheckError::StateUnavailable)
        ));
        storage
            .asm()
            .put_anchor_state_blocking(genesis, anchor_state(&checkpoint))
            .unwrap();
        assert_eq!(storage.l1().get_canonical_chain_tip().unwrap(), None);
        assert_eq!(read_canonical_checkpoint_blocking(&storage).unwrap(), None);
        let checked_block =
            check_startup_artifacts_blocking(&storage, &predicates, genesis).unwrap();
        assert_eq!(checked_block, genesis);
        let empty_predicates = CheckpointArtifactRegistry::<()>::empty().to_predicates();
        assert!(matches!(
            check_startup_artifacts_blocking(&storage, &empty_predicates, genesis),
            Err(CheckpointArtifactCheckError::MissingArtifacts(_))
        ));
        let other = L1BlockCommitment::new(1, L1BlockId::from(Buf32::from([2; 32])));
        assert!(matches!(
            check_startup_artifacts_blocking(&storage, &predicates, other),
            Err(CheckpointArtifactCheckError::StateUnavailable)
        ));
        let later = L1BlockCommitment::new(2, L1BlockId::from(Buf32::from([2; 32])));
        storage
            .asm()
            .put_anchor_state_blocking(later, anchor_state(&checkpoint))
            .unwrap();
        assert!(matches!(
            check_startup_artifacts_blocking(&storage, &predicates, genesis),
            Err(CheckpointArtifactCheckError::StateUnavailable)
        ));
    }

    #[test]
    fn startup_does_not_bypass_a_conflicting_canonical_index_with_genesis() {
        let storage = storage();
        let predicates = loaded_predicates();
        let checkpoint = checkpoint(&predicates);
        let genesis = L1BlockCommitment::new(1, L1BlockId::from(Buf32::from([1; 32])));
        let conflicting = L1BlockCommitment::new(1, L1BlockId::from(Buf32::from([2; 32])));
        storage
            .asm()
            .put_anchor_state_blocking(genesis, anchor_state(&checkpoint))
            .unwrap();
        storage
            .l1()
            .extend_canonical_chain(conflicting.blkid(), conflicting.height())
            .unwrap();
        assert!(matches!(
            check_startup_artifacts_blocking(&storage, &predicates, genesis),
            Err(CheckpointArtifactCheckError::StateUnavailable)
        ));
    }

    #[test]
    fn startup_accepts_preloaded_artifacts_before_and_after_enactment() {
        let storage = storage();
        let native = native_checkpoint_registry(OLRuntimeParams::test_default());
        let successor = native.get(OLSpecId::V1).unwrap();
        let previous_predicate = PredicateKey::always_accept();
        let mut registry = CheckpointArtifactRegistry::empty();
        // Unit hosts exercise artifact checks without claiming executable V0 proving
        // support or adding a production V2 variant. Bundle integrity has separate tests.
        registry
            .insert(CheckpointArtifact::new(
                OLSpecId::V0,
                previous_predicate.clone(),
                successor.program_id().clone(),
                (),
            ))
            .unwrap();
        let without_preload = registry.to_predicates();
        registry
            .insert(CheckpointArtifact::new(
                OLSpecId::V1,
                successor.predicate().clone(),
                successor.program_id().clone(),
                (),
            ))
            .unwrap();
        let predicates = registry.to_predicates();
        let mut checkpoint = checkpoint(&predicates);
        checkpoint.checkpoint_predicate = previous_predicate;
        let genesis = extend_snapshot(&storage, 1, &checkpoint);

        // The loaded predicates form a strict superset of the active/pending requirements.
        let startup = check_startup_artifacts_blocking(&storage, &predicates, genesis).unwrap();
        assert_eq!(predicates.iter().count(), 2);
        assert_eq!(startup, genesis);

        checkpoint.queue_predicate_transition(PendingPredicateTransition::new(
            successor.predicate().clone(),
            10,
        ));
        let enacted = extend_snapshot(&storage, 2, &checkpoint);
        let checked_block =
            check_startup_artifacts_blocking(&storage, &predicates, genesis).unwrap();
        assert_eq!(checked_block, enacted);
        let error =
            check_startup_artifacts_blocking(&storage, &without_preload, genesis).unwrap_err();
        let CheckpointArtifactCheckError::MissingArtifacts(error) = error else {
            panic!("expected missing pending artifact");
        };
        let [missing] = error.missing_artifacts() else {
            panic!("expected one missing artifact");
        };
        assert_eq!(
            missing.status(),
            CheckpointPredicateStatus::Pending { boundary: 10 }
        );
        assert_eq!(
            registry
                .into_hosts()
                .map(|(spec, _)| spec)
                .collect::<Vec<_>>(),
            vec![OLSpecId::V0, OLSpecId::V1],
            "artifact checks retain both resident hosts for fixed-spec services"
        );
    }

    #[test]
    fn startup_checks_local_asm_snapshot_without_waiting_for_l1_catchup() {
        let storage = storage();
        let predicates = loaded_predicates();
        let checkpoint = checkpoint(&predicates);
        let persisted = extend_snapshot(&storage, 1, &checkpoint);
        for height in 2..=5 {
            let block_id = L1BlockId::from(Buf32::from([height as u8; 32]));
            storage
                .l1()
                .extend_canonical_chain(&block_id, height)
                .unwrap();
        }
        // The L1 index has advanced; ASM has not materialized those later blocks yet.
        let checked_block =
            check_startup_artifacts_blocking(&storage, &predicates, persisted).unwrap();
        assert_eq!(checked_block, persisted);
        assert_eq!(
            storage.l1().get_canonical_chain_tip().unwrap().unwrap().0,
            5
        );
    }

    #[test]
    fn genesis_fallback_hands_off_when_a_canonical_snapshot_becomes_available() {
        let storage = storage();
        let predicates = loaded_predicates();
        let mut checkpoint = checkpoint(&predicates);
        let genesis = extend_snapshot(&storage, 1, &checkpoint);
        checkpoint.queue_predicate_transition(PendingPredicateTransition::new(
            PredicateKey::always_accept(),
            10,
        ));
        extend_snapshot(&storage, 2, &checkpoint);

        // The startup fallback may discover the L1 index after the original canonical
        // read found none. Resolve its current snapshot instead of using stale genesis.
        let (_, checkpoint) = read_genesis_checkpoint_blocking(&storage, genesis)
            .unwrap()
            .unwrap();
        let error = check_checkpoint_artifacts(&predicates, &checkpoint).unwrap_err();
        let [missing] = error.missing_artifacts() else {
            panic!("expected one missing artifact");
        };
        assert_eq!(
            missing.status(),
            CheckpointPredicateStatus::Pending { boundary: 10 }
        );
    }
}
