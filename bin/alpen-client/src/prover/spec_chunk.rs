//! Chunk-level [`ProofSpec`].
//!
//! Task = [`ChunkId`] (newtype-wrapped to attach the byte-encoding +
//! `Display` bounds that paas's `TaskKey` requires without polluting
//! the domain type). Program = [`EeChunkProgram`]; its `fetch_input`
//! reads chunk blocks + parent header + pre-state from EE storage and
//! assembles `ee_chunk_runtime::PrivateInput`.

use std::{fmt, sync::Arc};

use alpen_ee_common::{BatchStorage, ChunkId, ChunkWitnessStore, ExecBlockStorage};
use alpen_ee_database::EeNodeStorage;
use async_trait::async_trait;
use reth_primitives::Block;
use reth_primitives_traits::Block as _;
use rsp_primitives::genesis::Genesis;
use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use strata_ee_acct_types::{ExecBlock, ExecHeader};
use strata_ee_chain_types::{
    ChunkTransition, ExecHeaderSummary, ExecInputs, ExecOutputs, OutputMessage, OutputTransfer,
};
use strata_ee_chunk_runtime::{PrivateInput, RawBlockData, RawChunkData};
use strata_evm_ee::{EvmBlock, EvmBlockBody, EvmExecutionEnvironment, EvmHeader};
use strata_paas::{ProofSpec, ProverError as PaasError, ProverResult};
use strata_proofimpl_alpen_chunk::{EeChunkProgram, EeChunkProofInput};

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

/// Tag byte + the underlying `ChunkId`'s bytes.
const CHUNK_TASK_BYTES: usize = 1 + size_of::<ChunkId>();

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
/// - **`ChunkWitnessStore`** — pre-computed chunk-spanning sparse pre-state + per-block raw block
///   bytes, written at chunk-seal time by the batch builder. Read here in `fetch_input` as the
///   primary source. A missing record returns `TransientFailure` so paas retries with backoff —
///   gives an upstream backfill window before prover-core converts the task to permanent failure on
///   retry exhaustion.
/// - **`ExecBlockStorage`** — per-block `ExecBlockRecord` for authoritative `ExecInputs` /
///   `ExecOutputs`.
pub(crate) struct ChunkSpec {
    batch_storage: Arc<dyn BatchStorage>,
    storage: Arc<EeNodeStorage>,
    genesis: Genesis,
}

impl ChunkSpec {
    pub(crate) fn new(
        batch_storage: Arc<dyn BatchStorage>,
        storage: Arc<EeNodeStorage>,
        genesis: Genesis,
    ) -> Self {
        Self {
            batch_storage,
            storage,
            genesis,
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

        // 2. Read the pre-computed chunk witness. The batch builder writes this at chunk-seal time
        //    when state is at-tip; a missing record means seal-time extraction failed (operator
        //    will see a `warn!` from the batch builder) or the record was wiped, or a transient gap
        //    in the upstream `AccessedStateGenerator` exex is still being filled in. Return a
        //    transient failure so paas retries with backoff — if a backfill or the operator
        //    restores the record before `max_retries` exhausts, the chunk recovers. Otherwise the
        //    retry budget runs out and prover-core converts it to a permanent failure on its own.
        let witness = self
            .storage
            .get_chunk_witness(chunk_id)
            .await
            .map_err(|e| PaasError::Storage(format!("get_chunk_witness({chunk_id:?}): {e}")))?
            .ok_or_else(|| {
                PaasError::TransientFailure(format!(
                    "no chunk witness for {chunk_id:?} yet — seal-time extraction may still be \
                     in flight, gated on the AccessedStateGenerator exex catching up, or the \
                     record was deleted"
                ))
            })?;

        if witness.block_count() != block_hashes.len() {
            return Err(PaasError::PermanentFailure(format!(
                "chunk witness block count mismatch for {chunk_id:?}: chunk has {} blocks, witness has {}",
                block_hashes.len(),
                witness.block_count(),
            )));
        }

        // Decode RLP-encoded alloy types from the witness for the rest of the
        // assembly path. (The encoding is the inverse of what the batch
        // builder did via `ChunkWitnessRecord::new` in main.rs.)
        let prev_header: alloy_consensus::Header =
            alloy_rlp::decode_exact(&witness.prev_header_rlp[..]).map_err(|e| {
                PaasError::PermanentFailure(format!("decode prev_header_rlp for {chunk_id:?}: {e}"))
            })?;
        let alloy_blocks: Vec<Block> = witness
            .blocks_rlp
            .iter()
            .map(|b| alloy_rlp::decode_exact(&b[..]))
            .collect::<Result<_, _>>()
            .map_err(|e| {
                PaasError::PermanentFailure(format!("decode block RLP for {chunk_id:?}: {e}"))
            })?;
        let raw_partial_pre_state = witness.raw_partial_pre_state;

        // 3. Parent header — wrap in `EvmHeader` and encode via `strata_codec` for the guest
        //    (expects the varint length prefix).
        let parent_evm_header = EvmHeader::new(prev_header);
        let parent_blkid: Hash = parent_evm_header.compute_block_id();
        let raw_prev_header = encode_to_vec(&parent_evm_header)
            .map_err(|e| PaasError::PermanentFailure(format!("encode prev header: {e}")))?;

        // Integrity check: the witness was extracted for some specific
        // chunk; here we confirm its parent-header + per-block hashes
        // line up with the chunk we're proving. A mismatch means the
        // witness was generated against a different chunk (stale
        // record, key collision, on-disk corruption) and the rest of
        // the assembly would happily produce garbage. Cheap to do —
        // we already have the hashes computed for the assembly loop
        // below.
        if parent_blkid != chunk_id.prev_block() {
            return Err(PaasError::PermanentFailure(format!(
                "chunk witness prev-block hash mismatch for {chunk_id:?}: \
                 chunk expects {:?}, witness has {parent_blkid:?}",
                chunk_id.prev_block(),
            )));
        }
        for (idx, (expected_hash, alloy_block)) in
            block_hashes.iter().zip(&alloy_blocks).enumerate()
        {
            let computed: Hash = EvmHeader::new(alloy_block.header.clone()).compute_block_id();
            if computed != *expected_hash {
                return Err(PaasError::PermanentFailure(format!(
                    "chunk witness block hash mismatch for {chunk_id:?} at index {idx}: \
                     chunk has {expected_hash:?}, witness has {computed:?}"
                )));
            }
        }

        // 4. Build RawBlockData per block from ExecBlockRecord (inputs/outputs)
        //    + persisted alloy Blocks (EvmBlock encoding).
        let mut block_datas: Vec<RawBlockData> = Vec::with_capacity(block_hashes.len());
        let mut aggregated_inputs = ExecInputs::new_empty();
        let mut aggregated_outputs = ExecOutputs::new_empty();
        let mut tip_blkid = parent_blkid;
        // Safe placeholder values: `block_hashes` was checked non-empty above
        // and the witness block count must match it, so the loop below always
        // overwrites these with the terminal block's verified metadata.
        let mut tip_state_root = Hash::zero();
        let mut tip_exec_header_summary = ExecHeaderSummary::new(Vec::new());

        for (block_hash, alloy_block) in block_hashes.iter().zip(&alloy_blocks) {
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
            tip_state_root = block.get_header().get_state_root();
            tip_exec_header_summary = block.get_header().get_exec_header_summary();

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

        let chunk_transition = ChunkTransition::new(
            parent_blkid,
            tip_blkid,
            tip_state_root,
            tip_exec_header_summary,
            aggregated_inputs,
            aggregated_outputs,
        );

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
/// TODO(STR-3553): move to upstream `ExecOutputs::extend_from`; outputs
/// must be bit-identical across chunk execution -> DA blob -> pub_params.
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
