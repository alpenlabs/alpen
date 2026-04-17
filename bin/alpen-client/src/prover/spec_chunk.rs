//! Chunk-level [`ProofSpec`].
//!
//! Task = [`ChunkId`] (newtype-wrapped to attach the byte-encoding +
//! `Display` bounds that paas's `TaskKey` requires without polluting
//! the domain type). Program = [`EeChunkProgram`]; its `fetch_input`
//! reads chunk blocks + parent header + pre-state from EE storage and
//! assembles `ee_chunk_runtime::PrivateInput`.

use std::{fmt, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{BatchStorage, ChunkId};
use alpen_ee_database::EeNodeStorage;
use alpen_reth_witness::RangeWitnessData;
use async_trait::async_trait;
use rsp_primitives::genesis::Genesis;
use strata_acct_types::Hash;
use strata_ee_chain_types::{ExecInputs, ExecOutputs, OutputMessage, OutputTransfer};
use strata_paas::{ProofSpec, ProverError as PaasError, ProverResult};
use strata_proofimpl_alpen_chunk::{EeChunkProgram, EeChunkProofInput};

/// Type-erased range witness extractor.
///
/// Wraps [`alpen_reth_witness::RangeWitnessExtractor`] behind a closure
/// so `ChunkSpec` doesn't have to be generic over the reth provider.
/// Constructed in `main.rs` from the launched node's provider + evm config.
pub(crate) type RangeWitnessFn =
    dyn Fn(B256, B256) -> eyre::Result<RangeWitnessData> + Send + Sync;

/// Chunk-id-shaped task identifier for paas.
///
/// Newtype over [`ChunkId`] so we can attach the byte-encoding +
/// `Display` impls that paas's `TaskKey` blanket bounds require without
/// adding paas-specific traits to the domain type.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChunkTask(pub ChunkId);

impl fmt::Display for ChunkTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ChunkTask(prev={}, last={})",
            self.0.prev_block(),
            self.0.last_block()
        )
    }
}

/// Single-byte kind tag for [`ChunkTask`] encoding.
///
/// The chunk + acct provers share one sled prover-task tree; this tag
/// disambiguates chunk keys from batch keys so single-chunk batches
/// (same `(prev, last)` pair) can't collide.
pub(crate) const CHUNK_TASK_TAG: u8 = b'c';

/// Tag byte + `prev_block (32B) || last_block (32B)`.
const CHUNK_TASK_BYTES: usize = 1 + 64;

impl From<ChunkTask> for Vec<u8> {
    fn from(task: ChunkTask) -> Self {
        let mut buf = Vec::with_capacity(CHUNK_TASK_BYTES);
        buf.push(CHUNK_TASK_TAG);
        let prev: [u8; 32] = task.0.prev_block().into();
        let last: [u8; 32] = task.0.last_block().into();
        buf.extend_from_slice(&prev);
        buf.extend_from_slice(&last);
        buf
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ChunkTaskDecodeError {
    #[error("invalid ChunkTask byte length: expected {CHUNK_TASK_BYTES}, got {0}")]
    InvalidLength(usize),
    #[error("invalid ChunkTask tag byte: expected 0x{CHUNK_TASK_TAG:02x}, got 0x{0:02x}")]
    InvalidTag(u8),
}

impl TryFrom<Vec<u8>> for ChunkTask {
    type Error = ChunkTaskDecodeError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        if bytes.len() != CHUNK_TASK_BYTES {
            return Err(ChunkTaskDecodeError::InvalidLength(bytes.len()));
        }
        if bytes[0] != CHUNK_TASK_TAG {
            return Err(ChunkTaskDecodeError::InvalidTag(bytes[0]));
        }
        let mut prev = [0u8; 32];
        let mut last = [0u8; 32];
        prev.copy_from_slice(&bytes[1..33]);
        last.copy_from_slice(&bytes[33..]);
        Ok(ChunkTask(ChunkId::from_parts(
            Hash::from(prev),
            Hash::from(last),
        )))
    }
}

/// Chunk proof specification.
///
/// Two data sources per chunk:
/// - [`RangeWitnessExtractor`] (via [`RangeWitnessFn`]) — chunk-spanning
///   sparse pre-state + per-block alloy Blocks (for `EvmBlock` encoding).
/// - [`ExecBlockStorage`] — per-block `ExecBlockRecord` for authoritative
///   `ExecInputs` / `ExecOutputs`.
pub(crate) struct ChunkSpec {
    batch_storage: Arc<dyn BatchStorage>,
    storage: Arc<EeNodeStorage>,
    genesis: Genesis,
    range_witness_fn: Arc<RangeWitnessFn>,
}

impl ChunkSpec {
    pub(crate) fn new(
        batch_storage: Arc<dyn BatchStorage>,
        storage: Arc<EeNodeStorage>,
        genesis: Genesis,
        range_witness_fn: Arc<RangeWitnessFn>,
    ) -> Self {
        Self {
            batch_storage,
            storage,
            genesis,
            range_witness_fn,
        }
    }
}

#[async_trait]
impl ProofSpec for ChunkSpec {
    type Task = ChunkTask;
    type Program = EeChunkProgram;

    async fn fetch_input(&self, task: &Self::Task) -> ProverResult<EeChunkProofInput> {
        let _ = (&self.batch_storage, &self.storage, &self.genesis, &self.range_witness_fn);
        Err(PaasError::TransientFailure(format!(
            "ChunkSpec::fetch_input not implemented yet ({task})"
        )))
    }
}

/// Chunk-level aggregation of per-block [`ExecOutputs`].
///
/// TODO(STR-1369): move to upstream `ExecOutputs::extend_from`; outputs
/// must be bit-identical across chunk execution → DA blob → pub_params.
fn extend_exec_outputs(dst: &mut ExecOutputs, src: &ExecOutputs) {
    for t in src.output_transfers() {
        dst.add_transfer(OutputTransfer::new(t.dest(), t.value()));
    }
    for m in src.output_messages() {
        dst.add_message(OutputMessage::new(m.dest(), m.payload().clone()));
    }
}

/// Chunk-level aggregation of per-block [`ExecInputs`].
fn extend_exec_inputs(dst: &mut ExecInputs, src: &ExecInputs) {
    for d in src.subject_deposits() {
        dst.add_subject_deposit(d.clone());
    }
}
