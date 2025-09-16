use std::{collections::BTreeMap, sync::Arc};

use bitcoin::Block;
use strata_asm_common::{AnchorState, AsmLogEntry, AuxPayload, ChainViewState};
use strata_asm_spec::StrataAsmSpec;
use strata_asm_stf::{AsmStfInput, AsmStfOutput};
use strata_primitives::{
    l1::{HeaderVerificationState, L1BlockCommitment},
    params::Params,
};
use strata_service::ServiceState;

use crate::{WorkerContext, WorkerError, WorkerResult};

/// Service state for the ASM worker.
#[derive(Debug)]
pub struct AsmWorkerServiceState<W> {
    /// Params.
    pub(crate) params: Arc<Params>,

    /// Context for the state to interact with outer world.
    pub(crate) context: W,

    /// Whether the service is initialized.
    pub(crate) initialized: bool,

    /// ASM log entries emitted as a result of applying the current anchor block.
    #[expect(unused, reason = "will be used soon")]
    pub(crate) cur_logs: Option<Vec<AsmLogEntry>>,

    /// Current anchor state.
    pub(crate) anchor: Option<AnchorState>,

    /// Current anchor block.
    pub(crate) blkid: Option<L1BlockCommitment>,

    /// ASM spec for ASM STF.
    asm_spec: StrataAsmSpec,
}

impl<W: WorkerContext + Send + Sync + 'static> AsmWorkerServiceState<W> {
    /// A new (uninitialized) instance of the service state.
    pub(crate) fn new(context: W, params: Arc<Params>) -> Self {
        let asm_spec = StrataAsmSpec::from_params(params.rollup());
        Self {
            params,
            context,
            anchor: None,
            blkid: None,
            cur_logs: None,
            initialized: false,
            asm_spec,
        }
    }

    /// Loads and sets the latest anchor state.
    ///
    /// If there are no anchor states yet, creates and stores genesis one beforehand.
    pub(crate) fn load_latest_or_create_genesis(&mut self) -> WorkerResult<()> {
        match self.context.get_latest_asm_state()? {
            Some((blkid, state)) => {
                self.update_anchor_state(state, blkid);
                Ok(())
            }
            None => {
                // Create genesis anchor state.
                let genesis_l1_view = &self.params.rollup().genesis_l1_view;
                let state = AnchorState {
                    chain_view: ChainViewState {
                        pow_state: HeaderVerificationState::new(
                            self.context.get_network()?,
                            genesis_l1_view,
                        ),
                    },
                    sections: vec![],
                };

                // Persist it and update state.
                self.context
                    .store_anchor_state(&genesis_l1_view.blk, &state)?;
                self.update_anchor_state(state, genesis_l1_view.blk);

                Ok(())
            }
        }
    }

    /// Returns the actual ASM STF results: a Bitcoin block is applied onto current anchor state.
    ///
    /// A caller is responsible for ensuring the current anchor is a parent of a passed block.
    pub(crate) fn transition(&self, block: &Block) -> WorkerResult<AsmStfOutput> {
        let cur_state = self.anchor.as_ref().expect("state should be set before");

        // Pre process transition next block against current anchor state.
        let pre_process = strata_asm_stf::pre_process_asm(&self.asm_spec, cur_state, block)
            .map_err(WorkerError::AsmError)?;

        // Data transformation.
        let protocol_txs = pre_process
            .txs
            .into_iter()
            .map(|t| (t.tag().subproto_id(), t))
            .fold(BTreeMap::new(), |mut acc, (k, v)| {
                acc.entry(k).or_insert_with(Vec::new).push(v);
                acc
            });

        // TODO(QQ): not sure if it's correct.
        let aux_input = pre_process
            .aux_requests
            .iter()
            .map(|(k, v)| {
                (
                    *k,
                    AuxPayload {
                        data: v.data().to_vec(),
                    },
                )
            })
            .collect();

        let stf_input = AsmStfInput {
            protocol_txs,
            header: &block.header,
            aux_input: &aux_input,
        };

        // Asm transition.
        strata_asm_stf::compute_asm_transition(&self.asm_spec, cur_state, stf_input)
            .map_err(WorkerError::AsmError)
    }

    /// Updates anchor related bookkeping.
    pub(crate) fn update_anchor_state(&mut self, anchor: AnchorState, blkid: L1BlockCommitment) {
        self.initialized = true;
        self.anchor = Some(anchor);
        self.blkid = Some(blkid);
    }
}

impl<W: WorkerContext + Send + Sync + 'static> ServiceState for AsmWorkerServiceState<W> {
    fn name(&self) -> &str {
        "asm_worker"
    }
}
