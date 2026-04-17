//! Chunk-level [`ProofSpec`].
//!
//! Task = [`ChunkId`] (newtype-wrapped to attach the byte-encoding +
//! `Display` bounds that paas's `TaskKey` requires without polluting
//! the domain type). Program = [`EeChunkProgram`]; its `fetch_input`
//! reads chunk blocks + parent header + pre-state from EE storage and
//! assembles `ee_chunk_runtime::PrivateInput`.

use std::{fmt, sync::Arc};

use alloy_consensus::Header;
use alloy_primitives::B256;
use alloy_rlp::Decodable as _;
use alpen_ee_common::{BatchStorage, ChunkId, ExecBlockStorage};
use alpen_ee_database::EeNodeStorage;
use alpen_reth_witness::RangeWitnessData;
use async_trait::async_trait;
use reth_primitives_traits::Block as _;
use rsp_primitives::genesis::Genesis;
use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use strata_ee_acct_types::{ExecBlock, ExecHeader};
use strata_ee_chain_types::{ChunkTransition, ExecInputs, ExecOutputs, OutputMessage, OutputTransfer};
use strata_ee_chunk_runtime::{PrivateInput, RawBlockData, RawChunkData};
use strata_evm_ee::{EvmBlock, EvmBlockBody, EvmExecutionEnvironment, EvmHeader};
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
        let chunk_id = task.0;

        // 1. Read the chunk's block list.
        let (chunk, _status) = self
            .batch_storage
            .get_chunk_by_id(chunk_id)
            .await
            .map_err(|e| PaasError::Storage(format!("get_chunk_by_id({chunk_id:?}): {e}")))?
            .ok_or_else(|| {
                PaasError::TransientFailure(format!("chunk {chunk_id:?} not in storage"))
            })?;

        let block_hashes: Vec<Hash> = chunk.blocks_iter().collect();
        if block_hashes.is_empty() {
            return Err(PaasError::PermanentFailure(format!(
                "chunk {chunk_id:?} has no blocks"
            )));
        }

        let first_block_hash = B256::from(block_hashes[0].0);
        let last_block_hash = B256::from(block_hashes.last().unwrap().0);

        // 2. Extract chunk-spanning witness via RangeWitnessExtractor.
        //
        //    Produces a single sparse `EvmPartialState` covering the union
        //    of all blocks' read sets, projected onto the pre-chunk state,
        //    plus the alloy Blocks for `EvmBlock` encoding.
        //
        //    Suboptimal: re-executes blocks on every call. Should be
        //    pre-computed at chunk sealing time once the chunk builder
        //    exists — see comment in main.rs.
        let range_fn = self.range_witness_fn.clone();
        let range_data: RangeWitnessData = tokio::task::spawn_blocking(move || {
            (range_fn)(first_block_hash, last_block_hash)
        })
        .await
        .map_err(|e| PaasError::TransientFailure(format!("witness extraction join: {e}")))?
        .map_err(|e| PaasError::TransientFailure(format!("witness extraction: {e}")))?;

        let raw_partial_pre_state = range_data.raw_partial_pre_state;

        // 3. Parent header — decode from the range data's alloy RLP and
        //    re-encode through EvmHeader for strata_codec compat (the
        //    guest expects the varint length prefix).
        let parent_header = Header::decode(&mut range_data.raw_prev_header.as_slice()).map_err(|e| {
                PaasError::PermanentFailure(format!("decode range prev header: {e}"))
            })?;
        let parent_evm_header = EvmHeader::new(parent_header);
        let parent_blkid: Hash = parent_evm_header.compute_block_id();
        let raw_prev_header = encode_to_vec(&parent_evm_header)
            .map_err(|e| PaasError::PermanentFailure(format!("encode prev header: {e}")))?;

        // 4. Build RawBlockData per block from ExecBlockRecord (inputs/outputs)
        //    + range witness blocks (EvmBlock encoding).
        let mut block_datas: Vec<RawBlockData> = Vec::with_capacity(block_hashes.len());
        let mut aggregated_inputs = ExecInputs::new_empty();
        let mut aggregated_outputs = ExecOutputs::new_empty();
        let mut tip_blkid = parent_blkid;

        for (block_hash, alloy_block) in block_hashes.iter().zip(&range_data.blocks) {
            // Authoritative inputs/outputs from ExecBlockRecord.
            let record = self
                .storage
                .get_exec_block(*block_hash)
                .await
                .map_err(|e| PaasError::Storage(format!("get_exec_block({block_hash:?}): {e}")))?
                .ok_or_else(|| {
                    PaasError::TransientFailure(format!(
                        "ExecBlockRecord missing for {block_hash:?} in chunk {chunk_id:?}"
                    ))
                })?;
            let block_inputs = record.package().inputs().clone();
            let block_outputs = record.package().outputs().clone();

            // EvmBlock from the range witness's alloy Block.
            let evm_header = EvmHeader::new(alloy_block.header.clone());
            let body = EvmBlockBody::from_alloy_body(alloy_block.body().clone());
            let block = EvmBlock::new(evm_header, body);
            tip_blkid = block.get_header().compute_block_id();

            extend_exec_inputs(&mut aggregated_inputs, &block_inputs);
            extend_exec_outputs(&mut aggregated_outputs, &block_outputs);

            block_datas.push(
                RawBlockData::from_block::<EvmExecutionEnvironment>(
                    &block,
                    block_inputs,
                    block_outputs,
                )
                .map_err(|e| {
                    PaasError::PermanentFailure(format!("encode block {block_hash:?}: {e}"))
                })?,
            );
        }

        let chunk_transition =
            ChunkTransition::new(parent_blkid, tip_blkid, aggregated_inputs, aggregated_outputs);

        let raw_chunk = RawChunkData::new(block_datas, parent_blkid);
        let private_input = PrivateInput::new(
            chunk_transition,
            raw_chunk,
            raw_prev_header,
            raw_partial_pre_state,
        );

        Ok(EeChunkProofInput {
            genesis: self.genesis.clone(),
            private_input,
        })
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
