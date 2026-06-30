//! Chunk-level [`ProofSpec`].
//!
//! Task = [`ChunkId`] (newtype-wrapped to attach the byte-encoding +
//! `Display` bounds that paas's `TaskKey` requires without polluting
//! the domain type). Program = [`EeChunkProgram`]; its `fetch_input`
//! reads chunk blocks + parent header + pre-state from EE storage and
//! assembles `ee_chunk_runtime::PrivateInput`.

use std::{fmt, sync::Arc};

use alpen_ee_common::{
    decode_chunk_task_key, encode_chunk_task_key, BlockWitnessStore, ChunkId, ChunkStorage,
    ExecBlockStorage, ProverTaskKeyDecodeError,
};
use alpen_ee_database::EeNodeStorage;
use alpen_reth_node::BlockWitnessRecord;
use async_trait::async_trait;
use reth_primitives::Block;
use reth_primitives_traits::Block as _;
use rsp_primitives::genesis::Genesis;
use strata_acct_types::Hash;
use strata_bridge_params::BridgeParams;
use strata_codec::encode_to_vec;
use strata_ee_acct_types::{ExecBlock, ExecHeader};
use strata_ee_chain_types::{
    ChunkTransition, ExecHeaderSummary, ExecInputs, ExecOutputs, OutputMessage, OutputTransfer,
};
use strata_ee_chunk_runtime::{PrivateInput, RawBlockData, RawChunkData};
use strata_evm_ee::{EvmBlock, EvmBlockBody, EvmExecutionEnvironment, EvmHeader, EvmPartialState};
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

impl From<ChunkTask> for Vec<u8> {
    fn from(task: ChunkTask) -> Self {
        encode_chunk_task_key(task.0)
    }
}

impl TryFrom<Vec<u8>> for ChunkTask {
    type Error = ProverTaskKeyDecodeError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        decode_chunk_task_key(&bytes).map(ChunkTask)
    }
}

/// Chunk proof specification.
///
/// The input is assembled from per-block records, then collapsed into one
/// chunk-level pre-state:
/// - **`BlockWitnessStore`** — one [`BlockWitnessRecord`] per block (raw depth-0 witness parts:
///   trie-node bag, bytecodes, BLOCKHASH ancestor headers, plus the RLP block and parent header),
///   written inline in the block-production path. Their node bags are unioned into a single
///   chunk-level sparse pre-state anchored at the chunk's start root. A missing record returns
///   `TransientFailure` so paas retries with backoff.
/// - **`ExecBlockStorage`** — per-block `ExecBlockRecord` for authoritative `ExecInputs` /
///   `ExecOutputs`.
///
/// TODO(STR-3735): Once the paas-retries `resolve_input`/`Blocked` API lands,
/// "not produced yet" should map to `Blocked` instead of a fake transient failure —
/// block-production blocks on witness capture, so a present chunk should always resolve `Ready` on
/// the normal path.
pub(crate) struct ChunkSpec {
    chunk_storage: Arc<dyn ChunkStorage>,
    storage: Arc<EeNodeStorage>,
    genesis: Genesis,
    bridge_params: BridgeParams,
}

impl ChunkSpec {
    pub(crate) fn new(
        chunk_storage: Arc<dyn ChunkStorage>,
        storage: Arc<EeNodeStorage>,
        genesis: Genesis,
        bridge_params: BridgeParams,
    ) -> Self {
        Self {
            chunk_storage,
            storage,
            genesis,
            bridge_params,
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
            .chunk_storage
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

        // 2. Assemble per-block data and accumulate the chunk-level witness union. Each record
        //    (written inline at block production) carries the raw depth-0 witness parts (trie-node
        //    bag, bytecodes, BLOCKHASH ancestor headers) plus the RLP block and parent header. We
        //    union the node bags across the chunk and build one chunk-level pre-state below.
        let mut block_datas: Vec<RawBlockData> = Vec::with_capacity(block_hashes.len());
        let mut aggregated_inputs = ExecInputs::new_empty();
        let mut aggregated_outputs = ExecOutputs::new_empty();
        let mut prev_header: Option<alloy_consensus::Header> = None;
        // Safe placeholders: `block_hashes` is non-empty, so the loop always
        // overwrites these with the terminal block's verified metadata.
        let mut tip_blkid = Hash::zero();
        let mut tip_state_root = Hash::zero();
        let mut tip_exec_header_summary = ExecHeaderSummary::new_empty();

        // Chunk-level witness union (the first block to touch any node captures
        // it at its unmodified, i.e. chunk-start, version — so the union of the
        // per-block node bags fully covers the chunk's start state).
        let mut union_witness_state: Vec<Vec<u8>> = Vec::new();
        let mut union_codes: Vec<Vec<u8>> = Vec::new();
        let mut union_ancestor_headers_rlp: Vec<Vec<u8>> = Vec::new();

        for (idx, block_hash) in block_hashes.iter().enumerate() {
            // Per-block witness record: depth-0 witness + RLP block + parent
            // header. Missing means production-time capture hasn't landed yet
            // (or the record was deleted) — a transient failure so paas retries.
            // Once the paas-retries `resolve_input` API lands, "not produced
            // yet" should map to `Blocked` rather than a transient failure.
            let bytes = self
                .storage
                .get_block_witness(*block_hash)
                .await
                .map_err(|e| PaasError::Storage(format!("get_block_witness({block_hash:?}): {e}")))?
                .ok_or_else(|| {
                    PaasError::TransientFailure(format!(
                        "no block witness for {block_hash:?} in chunk {chunk_id:?} yet — \
                         block-production capture may still be in flight or the record was deleted"
                    ))
                })?;
            let record = BlockWitnessRecord::decode(&bytes).map_err(|e| {
                PaasError::PermanentFailure(format!(
                    "decode block witness record for {block_hash:?}: {e}"
                ))
            })?;

            // Decode the RLP block and confirm its hash matches the chunk's.
            let alloy_block: Block =
                alloy_rlp::decode_exact(&record.raw_block_rlp[..]).map_err(|e| {
                    PaasError::PermanentFailure(format!("decode block RLP for {block_hash:?}: {e}"))
                })?;
            let evm_header = EvmHeader::new(alloy_block.header.clone());
            let computed: Hash = evm_header.compute_block_id();
            if computed != *block_hash {
                return Err(PaasError::PermanentFailure(format!(
                    "block witness hash mismatch for chunk {chunk_id:?} at index {idx}: \
                     chunk has {block_hash:?}, witness has {computed:?}"
                )));
            }

            // The first block's parent header is the chunk's prev_header.
            if idx == 0 {
                prev_header = Some(
                    alloy_rlp::decode_exact(&record.raw_parent_header_rlp[..]).map_err(|e| {
                        PaasError::PermanentFailure(format!(
                            "decode parent header for {block_hash:?}: {e}"
                        ))
                    })?,
                );
            }

            // Authoritative inputs/outputs from the ExecBlockRecord.
            let exec_record = self
                .storage
                .get_exec_block(*block_hash)
                .await
                .map_err(|e| PaasError::Storage(format!("get_exec_block({block_hash:?}): {e}")))?
                .ok_or_else(|| {
                    PaasError::TransientFailure(format!(
                        "ExecBlockRecord missing for {block_hash:?} in chunk {chunk_id:?}"
                    ))
                })?;
            let block_inputs = exec_record.package().inputs().clone();
            let block_outputs = exec_record.package().outputs().clone();

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

            // Accumulate this block's raw witness parts into the chunk union.
            union_witness_state.extend(record.witness_state);
            union_codes.extend(record.codes);
            union_ancestor_headers_rlp.extend(record.ancestor_headers);
        }

        // 3. Parent header (from the first block's witness) — wrap + encode for the guest, and
        //    confirm it matches the chunk's prev_block.
        let prev_header = prev_header.expect("non-empty chunk has a first block");

        // Build the single chunk-level pre-state from the union of the per-block
        // witness node bags, anchored at the chunk's start (prev header) state
        // root. `from_witness_parts` reconstructs the sparse trie purely
        // in-memory from the node bag — no historical-state access, no OOM.
        let chunk_start_state_root = prev_header.state_root;
        let ancestor_headers: Vec<alloy_consensus::Header> = union_ancestor_headers_rlp
            .iter()
            .map(|raw| alloy_rlp::decode_exact(&raw[..]))
            .collect::<Result<_, _>>()
            .map_err(|e| {
                PaasError::PermanentFailure(format!("decode chunk ancestor header: {e}"))
            })?;
        let chunk_pre_state = EvmPartialState::from_witness_parts(
            union_witness_state,
            chunk_start_state_root,
            union_codes,
            ancestor_headers,
        );
        let raw_chunk_pre_state = encode_to_vec(&chunk_pre_state)
            .map_err(|e| PaasError::PermanentFailure(format!("encode chunk pre-state: {e}")))?;

        let parent_evm_header = EvmHeader::new(prev_header);
        let parent_blkid: Hash = parent_evm_header.compute_block_id();
        if parent_blkid != chunk_id.prev_block() {
            return Err(PaasError::PermanentFailure(format!(
                "chunk witness prev-block mismatch for {chunk_id:?}: \
                 chunk expects {:?}, witness has {parent_blkid:?}",
                chunk_id.prev_block(),
            )));
        }
        let raw_prev_header = encode_to_vec(&parent_evm_header)
            .map_err(|e| PaasError::PermanentFailure(format!("encode prev header: {e}")))?;

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
            raw_chunk_pre_state,
        );

        Ok(EeChunkProofInput {
            genesis: self.genesis.clone(),
            private_input,
            bridge_params: self.bridge_params,
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
