//! CSM worker service implementation.

use std::marker::PhantomData;

use strata_asm_worker::AsmWorkerStatus;
use strata_service::{Response, Service, SyncService};
use tracing::*;

use crate::{
    context::CsmWorkerContext, errors::CsmWorkerError, state::CsmWorkerState,
    status::CsmWorkerStatus,
};

/// CSM worker service that acts as a listener to ASM worker status updates.
///
/// This service monitors ASM worker and reacts to checkpoint logs emitted by the
/// checkpoint subprotocol. When ASM processes a checkpoint transaction, it emits
/// a `CheckpointTipUpdate` log which this service processes to update the client state.
///
/// The service follows the listener pattern - it passively observes ASM status updates
/// via the service framework's `StatusMonitorInput` without ASM being aware of it.
#[derive(Debug)]
pub struct CsmWorkerService<C> {
    _ctx: PhantomData<C>,
}

impl<C: CsmWorkerContext + 'static> Service for CsmWorkerService<C> {
    type State = CsmWorkerState<C>;
    type Msg = AsmWorkerStatus;
    type Status = CsmWorkerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        let clstate = &state.last_committed_state;
        CsmWorkerStatus {
            cur_block: state.recent_asm_blocks.last().copied(),
            last_processed_epoch: state.last_processed_epoch.map(|e| e as u64),
            last_confirmed_epoch: clstate.get_last_epoch(),
            last_finalized_epoch: clstate.get_declared_final_epoch(),
        }
    }
}

impl<C: CsmWorkerContext + 'static> SyncService for CsmWorkerService<C> {
    fn process_input(state: &mut Self::State, asm_status: Self::Msg) -> anyhow::Result<Response> {
        strata_common::check_bail_trigger(strata_common::BAIL_CSM_EVENT);

        // Extract the current block from ASM status
        let Some(asm_block) = asm_status.cur_block else {
            // ASM hasn't processed any blocks yet
            trace!("ASM status has no current block, skipping");
            return Ok(Response::Continue);
        };

        trace!("CSM is processing ASM logs.");

        if let Err(e) = state.process_asm_block(asm_block, asm_status.logs()) {
            // If it is reorg past finality, halt the service.
            if matches!(e, CsmWorkerError::ReorgPastFinality { .. }) {
                error!(%asm_block, err = ?e, "reorg past finality; shutting down CSM worker");
                return Ok(Response::ShouldExit);
            }
            error!(%asm_block, err = ?e, "Failed to process ASM block");
            return Ok(Response::Continue);
        }

        Ok(Response::Continue)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_asm_logs::CheckpointTipUpdate;
    use strata_asm_proto_checkpoint_types::CheckpointTip;
    use strata_asm_worker::{AsmState, AsmWorkerStatus};
    use strata_csm_types::{ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{Buf32, L1BlockId, OLBlockId};
    use strata_primitives::prelude::*;
    use strata_service::{Response, SyncService};
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use super::CsmWorkerService;
    use crate::{state::CsmWorkerState, test_utils::StubCtx};

    /// Builds a worker anchored at `last` (height `last_height`) whose ASM
    /// block processing fails on the L1 fetch, so no block ever commits.
    fn state_with_failing_block(
        last_height: L1Height,
        finality_depth: u32,
    ) -> (
        CsmWorkerState<StubCtx>,
        Arc<strata_storage::NodeStorage>,
        L1BlockCommitment,
    ) {
        let params = strata_test_utils_l2::gen_asm_params();
        let db = get_test_sled_backend();
        let pool = threadpool::ThreadPool::new(4);
        let storage = Arc::new(create_node_storage(db, pool).expect("create storage"));

        // Seed the genesis ClientState row keyed at `last`, so bootstrap
        // resolves `last_asm_block = last` with no prior finality.
        let last = L1BlockCommitment::new(last_height, L1BlockId::from(Buf32::from([7; 32])));
        storage
            .client_state()
            .put_update_blocking(
                &last,
                ClientUpdateOutput::new(ClientState::new(None, None), vec![]),
            )
            .expect("seed client state");

        let mut arbgen = ArbitraryGenerator::new();
        let status_channel = Arc::new(StatusChannel::new(
            arbgen.generate(),
            params.anchor.block,
            arbgen.generate(),
            None,
            None,
        ));

        // L1-fetch failure pins the failure to the checkpoint-tip log path;
        // any other error type could mask the property we want to assert.
        let ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            finality_depth,
            params.magic,
            params.anchor.block,
        )
        .with_l1_fetch_failure()
        // Keep `last` on the canonical chain so the incoming target is a pure
        // extension whose processing fails on the L1 fetch (not a reorg).
        .with_canonical_block(last_height, *last.blkid());

        let state = CsmWorkerState::init_from_context(ctx).expect("bootstrap state");

        (state, storage, last)
    }

    /// Build an `AsmWorkerStatus` for `block` carrying a single checkpoint-tip
    /// log. With `with_l1_fetch_failure`, that log triggers the failure path
    /// in `process_asm_block`.
    fn status_with_tip_log(block: L1BlockCommitment, epoch: u32) -> AsmWorkerStatus {
        let l2_commitment = OLBlockCommitment::new(
            epoch as u64 * 10,
            OLBlockId::from(Buf32::from([epoch as u8; 32])),
        );
        let tip = CheckpointTip {
            epoch,
            l1_height: block.height(),
            l2_commitment,
        };
        let log = strata_asm_common::AsmLogEntry::from_log(&CheckpointTipUpdate::new(tip))
            .expect("tip log");
        let anchor = make_anchor();
        let asm_state = AsmState::new(anchor, vec![log]);
        AsmWorkerStatus {
            is_initialized: true,
            cur_block: Some(block),
            cur_state: Some(asm_state),
        }
    }

    fn make_anchor() -> strata_asm_common::AnchorState {
        use bitcoin::Network;
        use strata_asm_common::{
            AnchorState, AsmHistoryAccumulatorState, ChainViewState, HeaderVerificationState,
        };
        use strata_btc_verification::L1Anchor;
        use strata_l1_txfmt::MagicBytes;

        let anchor = L1Anchor {
            block: L1BlockCommitment::default(),
            next_target: 0,
            epoch_start_timestamp: 0,
            network: Network::Bitcoin,
        };
        AnchorState {
            magic: AnchorState::magic_ssz(MagicBytes::from(*b"ALPN")),
            chain_view: ChainViewState {
                pow_state: HeaderVerificationState::init(anchor),
                history_accumulator: AsmHistoryAccumulatorState::new(0),
            },
            sections: Default::default(),
        }
    }

    /// Builds a worker whose committed tip is no longer on the canonical chain,
    /// with no canonical entry anywhere in the reorg window, so any incoming
    /// block forces a reorg that diverges past the finalized anchor.
    fn state_with_reorg_past_finality() -> (
        CsmWorkerState<StubCtx>,
        Arc<strata_storage::NodeStorage>,
        L1BlockCommitment,
    ) {
        let params = strata_test_utils_l2::gen_asm_params();
        let db = get_test_sled_backend();
        let pool = threadpool::ThreadPool::new(4);
        let storage = Arc::new(create_node_storage(db, pool).expect("create storage"));

        let last = L1BlockCommitment::new(100, L1BlockId::from(Buf32::from([7; 32])));
        storage
            .client_state()
            .put_update_blocking(
                &last,
                ClientUpdateOutput::new(ClientState::new(None, None), vec![]),
            )
            .expect("seed client state");

        let mut arbgen = ArbitraryGenerator::new();
        let status_channel = Arc::new(StatusChannel::new(
            arbgen.generate(),
            params.anchor.block,
            arbgen.generate(),
            None,
            None,
        ));

        // No canonical block registered at the tip's height, so `last` reads as
        // non-canonical and fork detection finds nothing in the window.
        let ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            3,
            params.magic,
            params.anchor.block,
        );

        let state = CsmWorkerState::init_from_context(ctx).expect("bootstrap state");
        (state, storage, last)
    }

    /// A reorg that diverges past the finalized anchor must halt the worker:
    /// `process_input` returns `Response::ShouldExit` rather than swallowing the
    /// error and continuing against an unsafe chain.
    #[test]
    fn process_input_exits_on_reorg_past_finality() {
        let (mut state, _storage, _last) = state_with_reorg_past_finality();

        // A divergent block at the tip's height triggers a same-height reorg;
        // with no canonical entry in the window it reaches past finality.
        let incoming = L1BlockCommitment::new(100, L1BlockId::from(Buf32::from([9; 32])));
        let status = status_with_tip_log(incoming, /* epoch */ 1);

        let response =
            <CsmWorkerService<StubCtx> as SyncService>::process_input(&mut state, status)
                .expect("process_input never errors — failures are surfaced as responses");
        assert!(matches!(response, Response::ShouldExit));
    }

    /// When `process_asm_block` fails, nothing commits: finality stays put and
    /// no row is persisted at the failing block, so bootstrap doesn't treat it
    /// as committed on restart.
    #[test]
    fn process_input_skips_finalization_when_process_asm_block_fails() {
        let last_height: L1Height = 100;
        let finality_depth: u32 = 3;
        let (mut state, storage, last) = state_with_failing_block(last_height, finality_depth);

        let target = L1BlockCommitment::new(last_height + 1, L1BlockId::from(Buf32::from([8; 32])));
        let status = status_with_tip_log(target, /* epoch */ 1);

        let response =
            <CsmWorkerService<StubCtx> as SyncService>::process_input(&mut state, status)
                .expect("process_input never errors — failures are swallowed");
        assert!(matches!(response, Response::Continue));

        // Finality must not have moved in-memory.
        assert_eq!(
            state.last_committed_state.get_declared_final_epoch(),
            None,
            "finality must not advance when process_asm_block failed"
        );
        // The cursor must stay pinned at the last committed block.
        assert_eq!(state.recent_asm_blocks.last(), Some(&last));
        // No ClientState row may exist at the failed block.
        assert!(
            storage
                .client_state()
                .get_update_blocking(&target)
                .expect("query client state")
                .is_none(),
            "no ClientState row should be persisted at the failed block"
        );
    }
}
