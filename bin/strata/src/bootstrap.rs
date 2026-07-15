//! One-shot promotion of a checkpoint-sync datadir into a sequencer history anchor.

use anyhow::{Context, Result, anyhow, bail};
use strata_asm_common::Subprotocol;
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
use strata_checkpoint_types::reconstruct_terminal_header;
use strata_identifiers::{Buf32, EpochCommitment};
use strata_node_context::NodeContext;
use strata_primitives::l1::compute_confirmation_depth;
use strata_storage::NodeStorage;
use tracing::info;
use tree_hash::{Sha256Hasher, TreeHash};

use crate::startup_checks::verify_anchor_summary_and_state;

pub(crate) fn validate_bootstrap_role(requested: bool, is_sequencer: bool) -> Result<()> {
    if requested && !is_sequencer {
        bail!("--bootstrap-from-checkpoint is only valid together with the sequencer role");
    }
    Ok(())
}

/// Validates and promotes the latest finalized checkpoint into an immutable history anchor.
pub(crate) fn promote_from_checkpoint(ctx: &NodeContext) -> Result<()> {
    promote_from_checkpoint_storage(
        ctx.storage().as_ref(),
        ctx.config().btcio.l1_reorg_safe_depth,
    )
}

fn promote_from_checkpoint_storage(storage: &NodeStorage, l1_reorg_safe_depth: u32) -> Result<()> {
    if let Some(history_base) = storage
        .ol_block()
        .get_history_base_blocking()
        .context("checkpoint promotion: failed to query existing OL history base")?
    {
        info!(%history_base, "checkpoint promotion already completed; leaving immutable history anchor unchanged");
        return Ok(());
    }

    let latest_client_state = storage
        .client_state()
        .fetch_most_recent_state()
        .context("checkpoint promotion: failed to fetch most recent client state")?
        .ok_or_else(|| {
            anyhow!("checkpoint promotion: nothing to promote: no client state is stored")
        })?;
    let anchor = latest_client_state
        .1
        .get_declared_final_epoch()
        .ok_or_else(|| {
            anyhow!(
                "checkpoint promotion: nothing to promote: the most recent client state has no declared final epoch"
            )
        })?;

    let (_, asm_state) = storage
        .fetch_canonical_asm_state_blocking()
        .context("checkpoint promotion: failed to fetch canonical ASM state")?
        .ok_or_else(|| anyhow!("checkpoint promotion: canonical ASM state is missing"))?;
    let checkpoint_state = asm_state
        .state()
        .find_section(<CheckpointSubprotocol as Subprotocol>::ID)
        .ok_or_else(|| {
            anyhow!("checkpoint promotion: canonical ASM state has no checkpoint section")
        })?
        .try_to_state::<CheckpointSubprotocol>()
        .context("checkpoint promotion: failed to decode canonical ASM checkpoint state")?;
    let verified_tip = checkpoint_state.verified_tip();
    let verified_anchor =
        EpochCommitment::from_terminal(verified_tip.epoch, *verified_tip.l2_commitment());

    if verified_anchor.epoch() > anchor.epoch() {
        let l1_ref = storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(verified_anchor)
            .with_context(|| {
                format!(
                    "checkpoint promotion: failed to query L1 reference for in-flight epoch {}",
                    verified_anchor.epoch()
                )
            })?
            .ok_or_else(|| {
                anyhow!(
                    "checkpoint promotion: verified in-flight epoch {} is missing its CheckpointL1Ref",
                    verified_anchor.epoch()
                )
            })?;
        let l1_tip_height = storage
            .l1()
            .get_chain_tip_height()
            .context("checkpoint promotion: failed to query the current canonical L1 tip")?
            .ok_or_else(|| {
                anyhow!(
                    "checkpoint promotion: cannot evaluate finality for in-flight epoch {} because the canonical L1 tip is missing",
                    verified_anchor.epoch()
                )
            })?;
        let required_depth = l1_reorg_safe_depth.max(1);
        let current_depth =
            compute_confirmation_depth(l1_ref.block_height(), l1_tip_height).unwrap_or(0);
        let blocks_remaining = required_depth.saturating_sub(current_depth);
        bail!(
            "checkpoint promotion: verified checkpoint epoch {epoch} is still in flight at L1 height {l1_height}; {blocks_remaining} L1 blocks remain to reach the reorg-safe bury depth of {required_depth}. Keep running in checkpoint-sync mode until it finalizes, then retry",
            epoch = verified_anchor.epoch(),
            l1_height = l1_ref.block_height(),
        );
    }
    if verified_anchor != anchor {
        bail!(
            "checkpoint promotion: ASM verified checkpoint tip {verified_anchor} does not equal declared final anchor {anchor}"
        );
    }

    let payload = storage
        .ol_checkpoint()
        .get_checkpoint_l1_observed_payload_blocking(anchor)
        .context("checkpoint promotion: failed to query observed checkpoint payload")?
        .ok_or_else(|| {
            anyhow!("checkpoint promotion: observed checkpoint payload is missing for {anchor}")
        })?;
    let l1_ref = storage
        .ol_checkpoint()
        .get_checkpoint_l1_ref_blocking(anchor)
        .context("checkpoint promotion: failed to query checkpoint L1 reference")?
        .ok_or_else(|| anyhow!("checkpoint promotion: CheckpointL1Ref is missing for {anchor}"))?;
    let canonical_l1_blkid = storage
        .l1()
        .get_canonical_blockid_at_height(l1_ref.block_height())
        .with_context(|| {
            format!(
                "checkpoint promotion: failed to query canonical L1 block at height {}",
                l1_ref.block_height()
            )
        })?;
    if canonical_l1_blkid != Some(*l1_ref.block_id()) {
        bail!(
            "checkpoint promotion: checkpoint L1 reference is not canonical at height {}: expected {}, got {:?}",
            l1_ref.block_height(),
            l1_ref.block_id(),
            canonical_l1_blkid,
        );
    }

    let summary = storage
        .ol_checkpoint()
        .get_epoch_summary_blocking(anchor)
        .context("checkpoint promotion: failed to query anchor epoch summary")?
        .ok_or_else(|| {
            anyhow!("checkpoint promotion: epoch summary is missing for anchor {anchor}")
        })?;
    let anchor_blkid = *anchor.last_blkid();
    let stored_header = storage
        .ol_block()
        .get_terminal_header_blocking(anchor_blkid)
        .context("checkpoint promotion: failed to query anchor terminal header")?
        .ok_or_else(|| {
            anyhow!(
                "checkpoint promotion: terminal header is missing for anchor {anchor}; run `strata-dbtool backfill-terminal-headers` and retry"
            )
        })?;
    let reconstructed_header = reconstruct_terminal_header(
        payload.new_tip(),
        payload.sidecar().terminal_header_complement(),
        *summary.final_state(),
    )
    .with_context(|| {
        format!("checkpoint promotion: failed to reconstruct terminal header for {anchor}")
    })?;
    if reconstructed_header != stored_header {
        bail!(
            "checkpoint promotion: reconstructed terminal header does not equal the stored terminal header for {anchor}"
        );
    }

    verify_anchor_summary_and_state(storage, anchor, &stored_header, &summary)?;
    let anchor_state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(anchor.to_block_commitment())
        .context("checkpoint promotion: failed to reload anchor OL state for root verification")?
        .expect("anchor state presence was verified before root verification");
    let computed_state_root: Buf32 =
        TreeHash::tree_hash_root::<Sha256Hasher>(anchor_state.as_ref()).into();
    if computed_state_root != *stored_header.state_root() {
        bail!(
            "checkpoint promotion: stored OL state root mismatch for {anchor}: computed {computed_state_root}, checkpoint commits to {}",
            stored_header.state_root()
        );
    }

    let anchor_slot = anchor.last_slot();
    let canonical_tip = storage
        .ol_block()
        .get_canonical_tip_blocking()
        .context("checkpoint promotion: failed to query canonical OL tip")?
        .ok_or_else(|| anyhow!("checkpoint promotion: canonical OL tip is missing"))?;
    if canonical_tip.slot() > anchor_slot {
        bail!(
            "checkpoint promotion: canonical OL tip {canonical_tip} is above anchor slot {anchor_slot}"
        );
    }

    let highest_block_slot = storage
        .ol_block()
        .get_highest_block_slot_blocking()
        .context("checkpoint promotion: failed to query highest full OL block slot")?;
    if let Some(highest_block_slot) = highest_block_slot
        && highest_block_slot > anchor_slot
    {
        bail!(
            "checkpoint promotion: full OL block records exist above anchor slot {anchor_slot} (highest block slot {})",
            highest_block_slot
        );
    }

    if let Some(high_watermark) = storage
        .ol_block()
        .get_block_high_watermark_blocking()
        .context("checkpoint promotion: failed to query OL block high watermark")?
    {
        bail!(
            "checkpoint promotion: OL block high watermark must be absent, but found {high_watermark}"
        );
    }

    storage
        .ol_block()
        .promote_to_history_anchor_blocking(anchor)
        .context("checkpoint promotion: failed to atomically publish the OL history anchor")?;

    info!(
        epoch = anchor.epoch(),
        slot = anchor.last_slot(),
        blkid = %anchor.last_blkid(),
        "promoted checkpoint-sync datadir to sequencer history anchor"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use bitcoin::Network;
    use strata_asm_common::{
        AnchorState, AsmHistoryAccumulatorState, ChainViewState, HeaderVerificationState,
        SectionState,
    };
    use strata_asm_params::CheckpointInitConfig;
    use strata_asm_proto_checkpoint::{CheckpointState, CheckpointSubprotocol};
    use strata_asm_proto_checkpoint_types::{
        CheckpointPayload, CheckpointSidecar, CheckpointTip, TerminalHeaderComplement,
    };
    use strata_btc_verification::L1Anchor;
    use strata_checkpoint_types::{EpochSummary, reconstruct_terminal_header};
    use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{
        Buf32, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId, RBuf32,
    };
    use strata_l1_txfmt::MagicBytes;
    use strata_ledger_types::{IStateAccessor, IStateAccessorMut};
    use strata_ol_params::OLParams;
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_predicate::PredicateKey;
    use strata_state::asm_state::AsmState;
    use strata_storage::{NodeStorage, create_node_storage};

    use super::*;
    use crate::{genesis::init_ol_genesis, startup_checks::verify_history_anchor};

    const REORG_SAFE_DEPTH: u32 = 6;
    const CHECKPOINT_L1_HEIGHT: u32 = 100;
    const L1_TIP_HEIGHT: u32 = 105;

    struct PromotionFixture {
        storage: NodeStorage,
        genesis: OLBlockCommitment,
        anchor: EpochCommitment,
        header: strata_ol_chain_types::OLBlockHeader,
        summary: EpochSummary,
        payload: CheckpointPayload,
        l1_ref: CheckpointL1Ref,
    }

    impl PromotionFixture {
        fn new() -> Self {
            Self::with_presence(true, true, true)
        }

        fn with_presence(
            store_client_state: bool,
            store_terminal_header: bool,
            store_anchor_state: bool,
        ) -> Self {
            let storage = create_node_storage(
                get_test_sled_backend(),
                strata_storage::test_runtime_handle(),
            )
            .expect("create test storage");
            let genesis_l1 = l1_commitment(0);
            let genesis_params = OLParams {
                last_l1_block: genesis_l1,
                ..Default::default()
            };
            let genesis =
                init_ol_genesis(&genesis_params, &storage).expect("initialize OL genesis");
            let genesis_block = storage
                .ol_block()
                .get_block_data_blocking(*genesis.blkid())
                .expect("read genesis block")
                .expect("genesis block exists");
            let genesis_state = storage
                .ol_state()
                .get_toplevel_ol_state_blocking(genesis)
                .expect("read genesis state")
                .expect("genesis state exists");
            let mut anchor_state = MemoryStateBaseLayer::new((*genesis_state).clone());
            anchor_state.set_cur_slot(1);
            anchor_state.set_cur_epoch(2);
            let anchor_state_root = anchor_state
                .compute_state_root()
                .expect("compute state root");

            let mut header = genesis_block.header().clone();
            header.slot = 1;
            header.epoch = 1;
            header.parent_blkid = OLBlockId::from(Buf32::from([7; 32]));
            header.state_root = anchor_state_root;
            header.flags.set_is_terminal(true);
            let anchor_block = header.compute_block_commitment();
            let anchor = EpochCommitment::from_terminal(1, anchor_block);

            let genesis_epoch = EpochCommitment::from_terminal(0, genesis);
            let genesis_summary = storage
                .ol_checkpoint()
                .get_epoch_summary_blocking(genesis_epoch)
                .expect("read genesis summary")
                .expect("genesis summary exists");
            let summary = genesis_summary.create_next_epoch_summary(
                anchor_block,
                l1_commitment(CHECKPOINT_L1_HEIGHT - 1),
                anchor_state_root,
            );
            storage
                .ol_checkpoint()
                .insert_epoch_summary_blocking(summary)
                .expect("insert anchor summary");

            if store_anchor_state {
                storage
                    .ol_state()
                    .put_toplevel_ol_state_blocking(anchor_block, anchor_state.into_inner())
                    .expect("insert anchor state");
            }
            if store_terminal_header {
                storage
                    .ol_block()
                    .put_terminal_header_blocking(*anchor.last_blkid(), header.clone())
                    .expect("insert terminal header");
            }

            let complement = TerminalHeaderComplement::new(
                header.timestamp(),
                *header.parent_blkid(),
                *header.body_root(),
                *header.logs_root(),
            );
            let sidecar =
                CheckpointSidecar::new(Vec::new(), Vec::new(), complement).expect("build sidecar");
            let tip = CheckpointTip::new(1, CHECKPOINT_L1_HEIGHT - 1, anchor_block);
            let payload = CheckpointPayload::new(tip, sidecar, Vec::new()).expect("build payload");
            let l1_ref = CheckpointL1Ref::new(
                l1_commitment(CHECKPOINT_L1_HEIGHT),
                RBuf32::from([1; 32]),
                RBuf32::from([2; 32]),
            );
            storage
                .ol_checkpoint()
                .put_checkpoint_l1_observation_blocking(anchor, payload.clone(), l1_ref.clone())
                .expect("insert observed checkpoint");

            extend_l1_chain(&storage, CHECKPOINT_L1_HEIGHT, L1_TIP_HEIGHT);
            let l1_tip = l1_commitment(L1_TIP_HEIGHT);
            if store_client_state {
                let checkpoint = L1Checkpoint::new(*payload.new_tip(), l1_ref.clone());
                storage
                    .client_state()
                    .put_update_blocking(
                        &l1_tip,
                        ClientUpdateOutput::new(
                            ClientState::new(Some(checkpoint.clone()), Some(checkpoint)),
                            Vec::new(),
                        ),
                    )
                    .expect("insert client state");
            }
            put_asm_checkpoint_state(&storage, l1_tip, *payload.new_tip());

            Self {
                storage,
                genesis,
                anchor,
                header,
                summary,
                payload,
                l1_ref,
            }
        }

        fn promote(&self) -> Result<()> {
            promote_from_checkpoint_storage(&self.storage, REORG_SAFE_DEPTH)
        }

        fn assert_failure(&self, expected: &str) {
            let canonical_before = self
                .storage
                .ol_block()
                .get_canonical_tip_blocking()
                .expect("read canonical tip before failure");
            let error = self.promote().expect_err("promotion must fail");
            assert!(
                format!("{error:#}").contains(expected),
                "expected {expected:?}, got {error:#}"
            );
            assert_eq!(
                self.storage
                    .ol_block()
                    .get_history_base_blocking()
                    .expect("read history base after failure"),
                None,
                "failed promotion must not write the marker"
            );
            assert_eq!(
                self.storage
                    .ol_block()
                    .get_canonical_tip_blocking()
                    .expect("read canonical tip after failure"),
                canonical_before,
                "failed promotion must not alter the canonical index"
            );
        }

        fn overwrite_anchor_state(&self, slot: u64, epoch: u32) {
            let state = self
                .storage
                .ol_state()
                .get_toplevel_ol_state_blocking(self.anchor.to_block_commitment())
                .expect("read anchor state")
                .expect("anchor state exists");
            let mut state = MemoryStateBaseLayer::new((*state).clone());
            state.set_cur_slot(slot);
            state.set_cur_epoch(epoch);
            self.storage
                .ol_state()
                .put_toplevel_ol_state_blocking(
                    self.anchor.to_block_commitment(),
                    state.into_inner(),
                )
                .expect("overwrite anchor state");
        }

        fn overwrite_anchor_state_root_only(&self) {
            let state = self
                .storage
                .ol_state()
                .get_toplevel_ol_state_blocking(self.anchor.to_block_commitment())
                .expect("read anchor state")
                .expect("anchor state exists");
            let mut state = MemoryStateBaseLayer::new((*state).clone());
            state.set_asm_recorded_epoch(EpochCommitment::new(
                99,
                99,
                OLBlockId::from(Buf32::from([0x99; 32])),
            ));
            self.storage
                .ol_state()
                .put_toplevel_ol_state_blocking(
                    self.anchor.to_block_commitment(),
                    state.into_inner(),
                )
                .expect("overwrite anchor state root");
        }
    }

    fn l1_blkid(height: u32) -> L1BlockId {
        L1BlockId::from(Buf32::from([height as u8; 32]))
    }

    fn l1_commitment(height: u32) -> L1BlockCommitment {
        L1BlockCommitment::new(height, l1_blkid(height))
    }

    fn extend_l1_chain(storage: &NodeStorage, start: u32, end: u32) {
        for height in start..=end {
            storage
                .l1()
                .extend_canonical_chain(&l1_blkid(height), height)
                .expect("extend canonical L1 chain");
        }
    }

    fn put_asm_checkpoint_state(
        storage: &NodeStorage,
        l1_block: L1BlockCommitment,
        verified_tip: CheckpointTip,
    ) {
        let init_config = CheckpointInitConfig {
            sequencer_predicate: PredicateKey::always_accept(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1_height: 0,
            genesis_ol_blkid: OLBlockId::null(),
        };
        let mut checkpoint_state = CheckpointState::init(init_config);
        checkpoint_state.verified_tip = verified_tip;
        let checkpoint_section =
            SectionState::from_state::<CheckpointSubprotocol>(&checkpoint_state)
                .expect("encode checkpoint section");
        let l1_anchor = L1Anchor {
            block: l1_block,
            next_target: 0,
            epoch_start_timestamp: 0,
            network: Network::Bitcoin,
        };
        let anchor_state = AnchorState {
            magic: AnchorState::magic_ssz(MagicBytes::from(*b"ALPN")),
            chain_view: ChainViewState {
                pow_state: HeaderVerificationState::init(l1_anchor),
                history_accumulator: AsmHistoryAccumulatorState::new(0),
            },
            sections: vec![checkpoint_section]
                .try_into()
                .expect("checkpoint section fits"),
        };
        storage
            .asm()
            .put_state_blocking(l1_block, AsmState::new(anchor_state, Vec::new()))
            .expect("store ASM checkpoint state");
    }

    #[test]
    fn happy_path_promotes_and_is_idempotent() {
        let fixture = PromotionFixture::new();

        fixture.promote().expect("promote checkpoint datadir");
        assert_eq!(
            fixture
                .storage
                .ol_block()
                .get_history_base_blocking()
                .expect("read history base"),
            Some(fixture.anchor)
        );
        assert_eq!(
            fixture
                .storage
                .ol_block()
                .get_canonical_tip_blocking()
                .expect("read canonical tip"),
            Some(fixture.anchor.to_block_commitment())
        );
        verify_history_anchor(&fixture.storage, fixture.anchor)
            .expect("subsequent startup anchor checks pass");

        let newer = EpochCommitment::new(2, 9, OLBlockId::from(Buf32::from([9; 32])));
        let newer_checkpoint = L1Checkpoint::new(
            CheckpointTip::new(2, L1_TIP_HEIGHT, newer.to_block_commitment()),
            CheckpointL1Ref::new(
                l1_commitment(L1_TIP_HEIGHT),
                RBuf32::from([3; 32]),
                RBuf32::from([4; 32]),
            ),
        );
        fixture
            .storage
            .client_state()
            .put_update_blocking(
                &L1BlockCommitment::new(L1_TIP_HEIGHT + 1, l1_blkid(L1_TIP_HEIGHT + 1)),
                ClientUpdateOutput::new(
                    ClientState::new(Some(newer_checkpoint.clone()), Some(newer_checkpoint)),
                    Vec::new(),
                ),
            )
            .expect("insert newer client state");

        fixture.promote().expect("idempotent promotion rerun");
        assert_eq!(
            fixture
                .storage
                .ol_block()
                .get_history_base_blocking()
                .expect("read immutable history base"),
            Some(fixture.anchor)
        );
    }

    #[test]
    fn missing_client_state_is_nothing_to_promote() {
        PromotionFixture::with_presence(false, true, true)
            .assert_failure("nothing to promote: no client state");
    }

    #[test]
    fn missing_declared_final_epoch_is_nothing_to_promote() {
        let fixture = PromotionFixture::new();
        let later = L1BlockCommitment::new(L1_TIP_HEIGHT + 1, l1_blkid(L1_TIP_HEIGHT + 1));
        fixture
            .storage
            .client_state()
            .put_update_blocking(
                &later,
                ClientUpdateOutput::new(ClientState::new(None, None), Vec::new()),
            )
            .expect("overwrite latest client state");
        fixture.assert_failure("no declared final epoch");
    }

    #[test]
    fn verified_tip_ahead_reports_bury_depth_countdown() {
        let fixture = PromotionFixture::new();
        let in_flight = EpochCommitment::new(2, 2, OLBlockId::from(Buf32::from([8; 32])));
        let in_flight_ref = CheckpointL1Ref::new(
            l1_commitment(104),
            RBuf32::from([5; 32]),
            RBuf32::from([6; 32]),
        );
        fixture
            .storage
            .ol_checkpoint()
            .put_checkpoint_l1_ref_blocking(in_flight, in_flight_ref)
            .expect("insert in-flight L1 ref");
        put_asm_checkpoint_state(
            &fixture.storage,
            l1_commitment(L1_TIP_HEIGHT),
            CheckpointTip::new(2, 103, in_flight.to_block_commitment()),
        );

        fixture.assert_failure(
            "verified checkpoint epoch 2 is still in flight at L1 height 104; 4 L1 blocks remain",
        );
    }

    #[test]
    fn missing_observed_payload_fails_distinctly() {
        let fixture = PromotionFixture::new();
        fixture
            .storage
            .ol_checkpoint()
            .del_checkpoint_l1_observed_payload_blocking(fixture.anchor)
            .expect("delete observed payload");
        fixture.assert_failure("observed checkpoint payload is missing");
    }

    #[test]
    fn missing_l1_ref_fails_distinctly() {
        let fixture = PromotionFixture::new();
        fixture
            .storage
            .ol_checkpoint()
            .del_checkpoint_l1_ref_blocking(fixture.anchor)
            .expect("delete L1 ref");
        fixture.assert_failure("CheckpointL1Ref is missing");
    }

    #[test]
    fn noncanonical_l1_ref_fails_distinctly() {
        let fixture = PromotionFixture::new();
        let orphaned_ref = CheckpointL1Ref::new(
            L1BlockCommitment::new(
                fixture.l1_ref.block_height(),
                L1BlockId::from(Buf32::from([0xee; 32])),
            ),
            fixture.l1_ref.txid,
            fixture.l1_ref.wtxid,
        );
        fixture
            .storage
            .ol_checkpoint()
            .put_checkpoint_l1_ref_blocking(fixture.anchor, orphaned_ref)
            .expect("overwrite L1 ref");
        fixture.assert_failure("checkpoint L1 reference is not canonical");
    }

    #[test]
    fn missing_terminal_header_points_to_backfill() {
        PromotionFixture::with_presence(true, false, true)
            .assert_failure("strata-dbtool backfill-terminal-headers");
    }

    #[test]
    fn terminal_header_reconstruction_mismatch_fails_distinctly() {
        let fixture = PromotionFixture::new();
        let complement = TerminalHeaderComplement::new(
            fixture.header.timestamp() + 1,
            *fixture.header.parent_blkid(),
            *fixture.header.body_root(),
            *fixture.header.logs_root(),
        );
        let sidecar =
            CheckpointSidecar::new(Vec::new(), Vec::new(), complement).expect("build sidecar");
        let mismatched_payload = CheckpointPayload::new(
            *fixture.payload.new_tip(),
            sidecar,
            fixture.payload.proof().to_vec(),
        )
        .expect("build mismatched payload");
        fixture
            .storage
            .ol_checkpoint()
            .put_checkpoint_l1_observation_blocking(
                fixture.anchor,
                mismatched_payload,
                fixture.l1_ref.clone(),
            )
            .expect("overwrite observed payload");

        fixture.assert_failure("failed to reconstruct terminal header");
    }

    #[test]
    fn summary_and_canonical_index_inconsistency_fails_distinctly() {
        let fixture = PromotionFixture::new();
        let conflicting = EpochSummary::new(
            fixture.anchor.epoch(),
            OLBlockCommitment::new(0, OLBlockId::from(Buf32::zero())),
            *fixture.summary.prev_terminal(),
            *fixture.summary.new_l1(),
            *fixture.summary.final_state(),
        );
        fixture
            .storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(conflicting)
            .expect("insert conflicting summary");
        fixture.assert_failure("canonical OL history anchor epoch commitment mismatch");
    }

    #[test]
    fn missing_anchor_state_fails_distinctly() {
        PromotionFixture::with_presence(true, true, false)
            .assert_failure("missing OL history anchor state");
    }

    #[test]
    fn bad_anchor_state_slot_fails_distinctly() {
        let fixture = PromotionFixture::new();
        fixture.overwrite_anchor_state(2, 2);
        fixture.assert_failure("state slot mismatch: expected 1, got 2");
    }

    #[test]
    fn bad_anchor_state_epoch_fails_distinctly() {
        let fixture = PromotionFixture::new();
        fixture.overwrite_anchor_state(1, 1);
        fixture.assert_failure("state epoch mismatch: expected 2, got 1");
    }

    #[test]
    fn stored_anchor_state_root_mismatch_fails_distinctly() {
        let fixture = PromotionFixture::new();
        fixture.overwrite_anchor_state_root_only();
        fixture.assert_failure("stored OL state root mismatch");
    }

    #[test]
    fn canonical_tip_above_anchor_fails_without_rewriting_index() {
        let fixture = PromotionFixture::new();
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(
                fixture.anchor.last_slot() + 1,
                vec![OLBlockId::from(Buf32::from([0xaa; 32]))],
            )
            .expect("advance canonical tip above anchor");
        fixture.assert_failure("canonical OL tip");
    }

    #[test]
    fn block_records_above_anchor_fail_distinctly() {
        let fixture = PromotionFixture::new();
        let genesis_block = fixture
            .storage
            .ol_block()
            .get_block_data_blocking(*fixture.genesis.blkid())
            .expect("read genesis block")
            .expect("genesis exists");
        let mut block = genesis_block;
        block.signed_header.header.slot = fixture.anchor.last_slot() + 1;
        fixture
            .storage
            .ol_block()
            .put_block_data_blocking(block)
            .expect("insert block above anchor");
        fixture.assert_failure("full OL block records exist above anchor slot");
    }

    #[test]
    fn block_high_watermark_present_fails_distinctly() {
        let fixture = PromotionFixture::new();
        let genesis_block = fixture
            .storage
            .ol_block()
            .get_block_data_blocking(*fixture.genesis.blkid())
            .expect("read genesis block")
            .expect("genesis exists");
        let mut block = genesis_block;
        block.signed_header.header.slot = fixture.anchor.last_slot();
        fixture
            .storage
            .ol_block()
            .put_block_data_with_high_watermark_blocking(block)
            .expect("insert block high watermark at anchor slot");
        fixture.assert_failure("OL block high watermark must be absent");
    }

    #[test]
    fn flag_without_sequencer_role_is_rejected() {
        let error = validate_bootstrap_role(true, false).expect_err("role guard must fail");
        assert!(
            error
                .to_string()
                .contains("only valid together with the sequencer role")
        );
        validate_bootstrap_role(false, false).expect("unused flag is valid");
        validate_bootstrap_role(true, true).expect("sequencer promotion is valid");
    }

    #[test]
    fn fixture_terminal_header_reconstructs_from_observed_payload() {
        let fixture = PromotionFixture::new();
        let reconstructed = reconstruct_terminal_header(
            fixture.payload.new_tip(),
            fixture.payload.sidecar().terminal_header_complement(),
            *fixture.summary.final_state(),
        )
        .expect("reconstruct fixture terminal header");
        assert_eq!(reconstructed, fixture.header);
    }
}
