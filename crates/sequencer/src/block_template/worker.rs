use std::{
    collections::{
        hash_map::Entry::{Occupied, Vacant},
        HashMap,
    },
    sync::Arc,
};

use strata_db::traits::Database;
use strata_eectl::engine::ExecEngineCtl;
use strata_primitives::params::{Params, RollupParams};
use strata_state::{
    block::L2BlockBundle,
    block_validation::verify_sequencer_signature,
    header::{L2BlockHeader, L2Header},
    id::L2BlockId,
};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::ShutdownGuard;
use tokio::sync::{mpsc, RwLock};
use tracing::*;

use super::{
    prepare_block, BlockCompletionData, BlockGenerationConfig, BlockTemplate, Error,
    FullBlockTemplate, TemplateManagerRequest,
};
use crate::utils::now_millis;

/// Container to pass context to worker
#[expect(missing_debug_implementations)]
pub struct WorkerContext<D, E> {
    params: Arc<Params>,
    // TODO remove
    _database: Arc<D>,
    storage: Arc<NodeStorage>,
    engine: Arc<E>,
    status_channel: StatusChannel,
}

impl<D, E> WorkerContext<D, E> {
    /// Create new worker context.
    pub const fn new(
        params: Arc<Params>,
        database: Arc<D>,
        storage: Arc<NodeStorage>,
        engine: Arc<E>,
        status_channel: StatusChannel,
    ) -> Self {
        Self {
            params,
            _database: database,
            storage,
            engine,
            status_channel,
        }
    }
}

/// Mutable worker state
#[derive(Debug, Default)]
pub struct WorkerState {
    /// templateid -> template
    pub(crate) pending_templates: HashMap<L2BlockId, FullBlockTemplate>,
    /// parent blockid -> templateid
    pub(crate) pending_by_parent: HashMap<L2BlockId, L2BlockId>,
}

impl WorkerState {
    pub(crate) fn insert_template(&mut self, template_id: L2BlockId, template: FullBlockTemplate) {
        let parent_blockid = *template.header().parent();
        if let Some(_existing) = self.pending_templates.insert(template_id, template) {
            warn!("existing pending block template overwritten: {template_id}");
        }
        self.pending_by_parent.insert(parent_blockid, template_id);
    }

    pub(crate) fn get_pending_block_template(
        &self,
        template_id: L2BlockId,
    ) -> Result<BlockTemplate, Error> {
        self.pending_templates
            .get(&template_id)
            .map(BlockTemplate::from_full_ref)
            .ok_or(Error::UnknownTemplateId(template_id))
    }

    pub(crate) fn get_pending_block_template_by_parent(
        &self,
        parent_block_id: L2BlockId,
    ) -> Result<BlockTemplate, Error> {
        let template_id = self
            .pending_by_parent
            .get(&parent_block_id)
            .ok_or(Error::UnknownTemplateId(parent_block_id))?;

        self.pending_templates
            .get(template_id)
            .map(BlockTemplate::from_full_ref)
            .ok_or(Error::UnknownTemplateId(*template_id))
    }
}

/// State of worker shared between worker task and handle.
pub type SharedState = Arc<RwLock<WorkerState>>;

/// Block template worker task.
pub fn worker<D, E>(
    shutdown: ShutdownGuard,
    ctx: WorkerContext<D, E>,
    state: SharedState,
    mut rx: mpsc::Receiver<TemplateManagerRequest>,
) -> anyhow::Result<()>
where
    D: Database,
    E: ExecEngineCtl,
{
    while let Some(request) = rx.blocking_recv() {
        match request {
            TemplateManagerRequest::GenerateBlockTemplate(config, response) => {
                if response
                    .send(generate_block_template(&ctx, &state, config))
                    .is_err()
                {
                    warn!("failed sending GenerateBlockTemplate result");
                }
            }
            TemplateManagerRequest::CompleteBlockTemplate(template_id, completion, response) => {
                if response
                    .send(complete_block_template(
                        ctx.params.rollup(),
                        &state,
                        template_id,
                        completion,
                    ))
                    .is_err()
                {
                    warn!(?template_id, "failed sending CompleteBlockTemplate result");
                }
            }
        };

        if shutdown.should_shutdown() {
            break;
        }
    }

    Ok(())
}

/// Generate new [`BlockTemplate`] according to provided [`BlockGenerationConfig`].
fn generate_block_template<D, E>(
    ctx: &WorkerContext<D, E>,
    state: &RwLock<WorkerState>,
    config: BlockGenerationConfig,
) -> Result<BlockTemplate, Error>
where
    D: Database,
    E: ExecEngineCtl,
{
    // check if we already have pending template for this parent block id
    if let Ok(template) = state
        .blocking_read()
        .get_pending_block_template_by_parent(config.parent_block_id())
    {
        return Ok(template);
    }

    let full_template = generate_block_template_inner(
        config,
        ctx.params.as_ref(),
        ctx.storage.as_ref(),
        ctx.engine.as_ref(),
        &ctx.status_channel,
    )?;

    let template = BlockTemplate::from_full_ref(&full_template);

    let template_id = full_template.get_blockid();

    state
        .blocking_write()
        .insert_template(template_id, full_template);

    Ok(template)
}

fn generate_block_template_inner(
    config: BlockGenerationConfig,
    params: &Params,
    storage: &NodeStorage,
    engine: &impl ExecEngineCtl,
    _status_channel: &StatusChannel,
) -> Result<FullBlockTemplate, Error> {
    // get parent block
    let parent_blkid = config.parent_block_id();
    let l2man = storage.l2();
    let parent = l2man
        .get_block_data_blocking(&parent_blkid)?
        .ok_or(Error::UnknownTemplateId(parent_blkid))?;

    let parent_ts = parent.header().timestamp();

    // TODO get and use chainstate

    // next slot idx
    let slot = parent.header().slot() + 1;

    // next block timestamp
    let ts = config.ts().unwrap_or_else(now_millis);

    // maintain min block_time
    if ts < parent_ts + params.rollup().block_time {
        Err(Error::TimestampTooEarly(ts))?;
    }

    let asm_span = debug_span!("asmblk", %slot, %parent_blkid);
    let _guard = asm_span.enter();

    // Actually put the template together.
    let (header, body, accessory) = prepare_block(
        slot,
        parent,
        ts,
        config.epoch_gas_limit(),
        storage,
        engine,
        params,
    )?;

    Ok(FullBlockTemplate::new(header, body, accessory))
}

/// Verify [`BlockCompletionData`] and create [`L2BlockBundle`] from template with provided id.
fn complete_block_template(
    rollup_params: &RollupParams,
    state: &RwLock<WorkerState>,
    template_id: L2BlockId,
    completion: BlockCompletionData,
) -> Result<L2BlockBundle, Error> {
    let mut state = state.blocking_write();
    match state.pending_templates.entry(template_id) {
        Vacant(entry) => Err(Error::UnknownTemplateId(entry.into_key())),
        Occupied(entry) => {
            if !check_completion_data(rollup_params, entry.get().header(), &completion) {
                Err(Error::InvalidSignature(template_id))
            } else {
                let template = entry.remove();
                let parent = template.header().parent();
                state.pending_by_parent.remove(parent);
                Ok(template.complete_block_template(completion))
            }
        }
    }
}

fn check_completion_data(
    rollup_params: &RollupParams,
    header: &L2BlockHeader,
    completion: &BlockCompletionData,
) -> bool {
    // currently only checks for correct sequencer signature.
    verify_sequencer_signature(rollup_params, &header.get_sighash(), completion.signature())
}
