//! Acct (outer/update) [`ProofSpec`].
//!
//! Task = [`BatchId`] (newtype-wrapped). Program = [`EeAcctProgram`];
//! `fetch_input` reads chunk receipts (from the shared paas
//! `ReceiptStore`) and the prior batch's end-state, then assembles
//! `ee_acct_runtime::PrivateInput` + `snark_acct_runtime::PrivateInput`.
//!
//! `LedgerRefs` are derived from the batch's reduced L1 block refs
//! (`{block_hash, wtxids_root}` at the L1 height index) using the same helper
//! as the OL submitter, so the submitted update and proof pub-params stay
//! byte-identical.

use std::{fmt, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{
    build_ledger_refs_from_da, AccessedStateStore, BatchId, BatchStatus, BatchStorage,
    ExecBlockStorage, L1DaBlockRef, Storage,
};
use alpen_ee_database::EeNodeStorage;
use alpen_reth_db::StateDiffProvider;
use alpen_reth_statediff::{BatchBuilder, BatchStateDiff};
use alpen_reth_witness::RangeWitnessData;
use async_trait::async_trait;
use bitcoin::{consensus::serialize as btc_serialize, hashes::Hash as _};
use bitcoind_async_client::{traits::Reader, Client as BtcClient};
use rsp_primitives::genesis::Genesis;
use ssz::{Decode, Encode as _};
use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{
    ChunkInput, DaBlockWitness, DaBytecodeWitness, DaTxWitness, DaWitness, EePrivateInput,
    L1DaBlockInclusion,
};
use strata_ee_acct_types::UpdateExtraData;
use strata_ee_chain_types::ChunkTransition;
use strata_identifiers::{Buf32, WtxidsRoot};
use strata_paas::{ProofSpec, ProverError as PaasError, ProverResult, ReceiptStore};
use strata_primitives::l1::L1BlockIdBitcoinExt;
use strata_proofimpl_alpen_acct::{EeAcctProgram, EeAcctProofInput};
use strata_snark_acct_runtime::{Coinput, IInnerState, PrivateInput as UpdatePrivateInput};
use strata_snark_acct_types::{
    OutputMessage, OutputTransfer, ProofState, UpdateOutputs, UpdateProofPubParams,
};
use tokio::task;

use super::{
    da_witness_build::{
        build_wtxid_inclusion_proof, compute_wtxids_root, known_bytecodes_from_unfiltered_diff,
        reassemble_da_blob_from_txs,
    },
    ChunkTask,
};

pub(crate) type AcctRangeWitnessFn =
    dyn Fn(B256, B256) -> eyre::Result<RangeWitnessData> + Send + Sync;

/// Batch-id-shaped task identifier for paas. Newtype over [`BatchId`]
/// for the same reasons [`super::ChunkTask`] wraps `ChunkId`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BatchTask(pub BatchId);

impl fmt::Display for BatchTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Single-byte kind tag for [`BatchTask`] encoding; see the matching
/// `CHUNK_TASK_TAG` on `ChunkTask` for why the shared prover-task tree
/// needs a discriminator.
pub(crate) const BATCH_TASK_TAG: u8 = b'a';

/// Tag byte + the underlying `BatchId`'s bytes.
const BATCH_TASK_BYTES: usize = 1 + size_of::<BatchId>();

impl From<BatchTask> for Vec<u8> {
    fn from(task: BatchTask) -> Self {
        let mut buf = Vec::with_capacity(BATCH_TASK_BYTES);
        buf.push(BATCH_TASK_TAG);
        let prev: [u8; 32] = task.0.prev_block().into();
        let last: [u8; 32] = task.0.last_block().into();
        buf.extend_from_slice(&prev);
        buf.extend_from_slice(&last);
        buf
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BatchTaskDecodeError {
    #[error("invalid BatchTask byte length: expected {BATCH_TASK_BYTES}, got {0}")]
    InvalidLength(usize),
    #[error("invalid BatchTask tag byte: expected 0x{BATCH_TASK_TAG:02x}, got 0x{0:02x}")]
    InvalidTag(u8),
}

impl TryFrom<Vec<u8>> for BatchTask {
    type Error = BatchTaskDecodeError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        if bytes.len() != BATCH_TASK_BYTES {
            return Err(BatchTaskDecodeError::InvalidLength(bytes.len()));
        }
        if bytes[0] != BATCH_TASK_TAG {
            return Err(BatchTaskDecodeError::InvalidTag(bytes[0]));
        }
        let mut prev = [0u8; 32];
        let mut last = [0u8; 32];
        prev.copy_from_slice(&bytes[1..33]);
        last.copy_from_slice(&bytes[33..]);
        Ok(BatchTask(BatchId::from_parts(
            Hash::from(prev),
            Hash::from(last),
        )))
    }
}

#[derive(Debug, thiserror::Error)]
enum AcctProofInputError {
    #[error("failed to read parent exec block (parent {parent_blkid:?}, {reason})")]
    ReadParentExecBlock { parent_blkid: Hash, reason: String },

    #[error("failed to read best EE account state ({reason})")]
    ReadBestEeAccountState { reason: String },

    #[error("no EE account state available yet (genesis not loaded?)")]
    NoEeAccountState,

    #[error(
        "EE pre-state unavailable for batch {batch_id} (missing parent {parent_blkid:?}, OL-accepted tip {ol_accepted_tip:?})"
    )]
    EePreStateUnavailable {
        batch_id: BatchId,
        parent_blkid: Hash,
        ol_accepted_tip: Hash,
    },

    #[error("batch {batch_id} does not have DA refs yet (status {status})")]
    BatchMissingDaRefs {
        batch_id: BatchId,
        status: &'static str,
    },
}

impl From<AcctProofInputError> for PaasError {
    fn from(error: AcctProofInputError) -> Self {
        let message = error.to_string();
        match error {
            AcctProofInputError::ReadParentExecBlock { .. }
            | AcctProofInputError::ReadBestEeAccountState { .. } => PaasError::Storage(message),
            AcctProofInputError::NoEeAccountState
            | AcctProofInputError::EePreStateUnavailable { .. }
            | AcctProofInputError::BatchMissingDaRefs { .. } => {
                PaasError::TransientFailure(message)
            }
        }
    }
}

/// Outer-proof specification.
///
/// Holds the shared paas `ReceiptStore` (chunk receipts the chunk
/// prover wrote), `Arc<dyn BatchStorage>` for batch metadata,
/// `Arc<EeNodeStorage>` for `ExecBlockRecord` + `EeAccountState`
/// reads, and `Arc<EeBatchProofDbManager>` so the struct can be
/// shared with the receipt hook (which writes outer proofs there).
#[derive(Clone)]
pub(crate) struct AcctSpec {
    chunk_receipts: Arc<dyn ReceiptStore>,
    batch_storage: Arc<dyn BatchStorage>,
    storage: Arc<EeNodeStorage>,
    btc_client: Arc<BtcClient>,
    state_diff_provider: Arc<dyn StateDiffProvider + Send + Sync>,
    range_witness_fn: Arc<AcctRangeWitnessFn>,
    genesis: Genesis,
}

impl AcctSpec {
    pub(crate) fn new(
        chunk_receipts: Arc<dyn ReceiptStore>,
        batch_storage: Arc<dyn BatchStorage>,
        storage: Arc<EeNodeStorage>,
        btc_client: Arc<BtcClient>,
        state_diff_provider: Arc<dyn StateDiffProvider + Send + Sync>,
        range_witness_fn: Arc<AcctRangeWitnessFn>,
        genesis: Genesis,
    ) -> Self {
        Self {
            chunk_receipts,
            batch_storage,
            storage,
            btc_client,
            state_diff_provider,
            range_witness_fn,
            genesis,
        }
    }
}

#[async_trait]
impl ProofSpec for AcctSpec {
    type Task = BatchTask;
    type Program = EeAcctProgram;

    async fn fetch_input(&self, task: &Self::Task) -> ProverResult<EeAcctProofInput> {
        let batch_id = task.0;

        // 1. Chunk inputs: per-chunk transitions + their proofs, in order.
        let chunks: Vec<ChunkInput> =
            collect_chunk_inputs_for_batch(&*self.batch_storage, &*self.chunk_receipts, batch_id)
                .await?;
        if chunks.is_empty() {
            return Err(PaasError::PermanentFailure(format!(
                "batch {batch_id} has no chunks"
            )));
        }

        // 2. Batch metadata. The first block is the one immediately after `prev_block`; we resolve
        //    it via the FIRST chunk's parent_blkid.
        let (batch, status) = self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await
            .map_err(|e| PaasError::Storage(format!("get_batch_by_id({batch_id}): {e}")))?
            .ok_or_else(|| {
                PaasError::PermanentFailure(format!("batch {batch_id} not in storage"))
            })?;
        let da_refs = da_refs_from_status(batch_id, status)?;
        let update_seq_no = batch.update_seq_no().ok_or_else(|| {
            PaasError::PermanentFailure(format!("batch {batch_id} has no update seq_no"))
        })?;

        // 3. Previous EE account state.
        //
        //    We need the EE account state as it was JUST BEFORE this
        //    batch's first block. There are two ways to read it:
        //
        //      (a) `best_ee_account_state()` — the last OL-accepted state.
        //          Cheap and correct only when the batch lifecycle proves
        //          batches strictly serially: by the time batch N is in
        //          ProofReady, batch N-1's SAU has already landed on OL.
        //
        //      (b) The local `ExecBlockRecord` for this batch's first
        //          block's parent — its `account_state()` is the
        //          authoritative post-state of that parent, which is
        //          exactly the pre-state for our batch.
        //
        //    (b) is robust to batch pipelining (multiple batches proving
        //          concurrently), faster prover backends (where batch N
        //          can hit `fetch_input` before batch N-1's SAU has been
        //          submitted, applied on OL, and observed by the
        //          tracker), and recovery scenarios. We use (b) and only
        //          fall back to (a) when the parent record isn't in
        //          local storage (genesis, or an unrelated bug surfacing
        //          a missing record).
        let first_chunk = chunks.first().ok_or_else(|| {
            PaasError::PermanentFailure("first chunk missing after non-empty check".to_string())
        })?;
        let first_transition = decode_chunk_transition(first_chunk)?;
        let parent_blkid = first_transition.parent_exec_blkid();

        let pre_ee_state = match self
            .storage
            .get_exec_block(parent_blkid)
            .await
            .map_err(|e| AcctProofInputError::ReadParentExecBlock {
                parent_blkid,
                reason: e.to_string(),
            })? {
            Some(parent_record) => parent_record.account_state().clone(),
            None => {
                // Parent record not in local storage; fall back to the
                // OL-accepted state. Expected at genesis; otherwise
                // surface as transient (the alpen-client may still be
                // populating its local exec store).
                let acct_at_epoch = self
                    .storage
                    .best_ee_account_state()
                    .await
                    .map_err(|e| AcctProofInputError::ReadBestEeAccountState {
                        reason: e.to_string(),
                    })?
                    .ok_or(AcctProofInputError::NoEeAccountState)?;
                let fallback = acct_at_epoch.ee_state().clone();
                if fallback.last_exec_blkid() != parent_blkid {
                    return Err(AcctProofInputError::EePreStateUnavailable {
                        batch_id,
                        parent_blkid,
                        ol_accepted_tip: fallback.last_exec_blkid(),
                    }
                    .into());
                }
                fallback
            }
        };

        // 4. ee_acct private input.
        //
        //    The acct guest uses this sparse pre-state to apply the DA blob's
        //    batch state diff and compare the result with the final chunk's
        //    tip state root. This spans the full batch, not just one chunk.
        let batch_block_hashes: Vec<Hash> = batch.blocks_iter().collect();
        let first_batch_block = batch_block_hashes.first().copied().ok_or_else(|| {
            PaasError::PermanentFailure(format!("batch {batch_id} has no execution blocks"))
        })?;
        let first_block_hash = B256::from(first_batch_block.0);
        let last_block_hash = B256::from(batch.last_block().0);
        let range_witness_fn = self.range_witness_fn.clone();
        let range_data =
            task::spawn_blocking(move || (range_witness_fn)(first_block_hash, last_block_hash))
                .await
                .map_err(|e| {
                    PaasError::TransientFailure(format!("batch witness extraction join: {e}"))
                })?
                .map_err(|e| {
                    PaasError::TransientFailure(format!("batch witness extraction: {e}"))
                })?;

        let ee_private_input =
            EePrivateInput::new(Vec::new(), range_data.raw_partial_pre_state, chunks.clone());

        // 5. Build UpdateProofPubParams from ExecBlockRecords.
        //
        //    Mirrors the submission-side `update_builder.rs` construction:
        //    read each block's ExecBlockRecord to derive processed_inputs,
        //    messages, outputs, next_inbox_msg_idx, and new_tip_blkid.
        //    This is the authoritative source (same data the update
        //    submitter reads), so proof-input and submission agree.
        let mut processed_inputs: u32 = 0;
        let mut messages = Vec::new();
        let mut update_outputs = UpdateOutputs::new_empty();
        for block_hash in &batch_block_hashes {
            let record = self
                .storage
                .get_exec_block(*block_hash)
                .await
                .map_err(|e| PaasError::Storage(format!("get_exec_block({block_hash:?}): {e}")))?
                .ok_or_else(|| {
                    PaasError::TransientFailure(format!(
                        "ExecBlockRecord missing for {block_hash:?} in batch {batch_id}"
                    ))
                })?;

            let (package, _account_state, mut block_messages) = record.into_parts();
            processed_inputs += package.inputs().total_inputs() as u32;
            messages.append(&mut block_messages);
            update_outputs
                .try_extend_transfers(
                    package
                        .outputs()
                        .output_transfers()
                        .iter()
                        .map(|t| OutputTransfer::new(t.dest(), t.value())),
                )
                .map_err(|_| {
                    PaasError::PermanentFailure("UpdateOutputs transfers overflow".to_string())
                })?;
            update_outputs
                .try_extend_messages(
                    package
                        .outputs()
                        .output_messages()
                        .iter()
                        .map(|m| OutputMessage::new(m.dest(), m.payload().clone())),
                )
                .map_err(|_| {
                    PaasError::PermanentFailure("UpdateOutputs messages overflow".to_string())
                })?;
        }

        // Last block gives us post-batch metadata.
        let last_block_hash = batch.last_block();
        let last_record = self
            .storage
            .get_exec_block(last_block_hash)
            .await
            .map_err(|e| PaasError::Storage(format!("get_exec_block({last_block_hash:?}): {e}")))?
            .ok_or_else(|| {
                PaasError::PermanentFailure(format!(
                    "last block record missing for batch {batch_id}"
                ))
            })?;
        let new_tip_blkid = last_record.package().exec_blkid();
        let new_tip_state_root = last_record.account_state().last_exec_state_root();
        let new_inbox_idx = last_record.next_inbox_msg_idx();
        let post_state_root = last_record.account_state().compute_state_root();
        let message_count = messages.len() as u64;
        let pre_inbox_idx = new_inbox_idx.checked_sub(message_count).ok_or_else(|| {
            PaasError::TransientFailure(format!(
                "inconsistent inbox indices for batch {batch_id}: \
                 new_inbox_idx={new_inbox_idx}, message_count={message_count}"
            ))
        })?;

        // The post-state root must match the actual state stored with the
        // batch's last block. The account proof guest reaches this state by
        // applying messages, verifying chunks, and removing consumed pending
        // inputs from `pre_ee_state`; `UpdateExtraData` separately carries
        // the execution state root needed by EE reconstruction.
        let cur_state = ProofState::new(pre_ee_state.compute_state_root(), pre_inbox_idx);
        let new_state = ProofState::new(post_state_root, new_inbox_idx);

        let extra_data =
            UpdateExtraData::new(new_tip_blkid, new_tip_state_root, processed_inputs, 0);
        let extra_data_bytes = encode_to_vec(&extra_data)
            .map_err(|e| PaasError::PermanentFailure(format!("encode extra data: {e}")))?;
        let ledger_refs = build_ledger_refs_from_da(&da_refs);

        let pub_params = UpdateProofPubParams::new(
            update_seq_no,
            cur_state,
            new_state,
            messages,
            ledger_refs,
            update_outputs,
            extra_data_bytes,
        );

        // Coinputs: one empty coinput per message (EE program requires
        // empty coinputs — see verify_coinput in ee_program.rs).
        let coinputs = pub_params
            .message_inputs()
            .iter()
            .map(|_| Coinput::new(Vec::new()))
            .collect();

        let update_private_input =
            UpdatePrivateInput::new(pub_params, pre_ee_state.as_ssz_bytes(), coinputs);
        let da_witness = build_da_witness(
            &da_refs,
            &batch_block_hashes,
            &*self.btc_client,
            &*self.state_diff_provider,
            &*self.storage,
        )
        .await?;

        Ok(EeAcctProofInput {
            genesis: self.genesis.clone(),
            ee_private_input,
            update_private_input,
            da_witness,
        })
    }
}

fn da_refs_from_status(batch_id: BatchId, status: BatchStatus) -> ProverResult<Vec<L1DaBlockRef>> {
    match status {
        BatchStatus::DaComplete { da }
        | BatchStatus::ProofPending { da }
        | BatchStatus::ProofReady { da, .. } => Ok(da),
        BatchStatus::Genesis => Err(AcctProofInputError::BatchMissingDaRefs {
            batch_id,
            status: "genesis",
        }
        .into()),
        BatchStatus::Sealed => Err(AcctProofInputError::BatchMissingDaRefs {
            batch_id,
            status: "sealed",
        }
        .into()),
        BatchStatus::DaPending { .. } => Err(AcctProofInputError::BatchMissingDaRefs {
            batch_id,
            status: "DA pending",
        }
        .into()),
    }
}

/// Builds raw DA transaction witnesses from the L1 blocks referenced by batch DA refs.
async fn build_da_witness(
    da_refs: &[L1DaBlockRef],
    batch_block_hashes: &[Hash],
    btc: &(impl Reader + Sync),
    state_diff_provider: &(impl StateDiffProvider + Sync + ?Sized),
    bytecode_store: &impl AccessedStateStore,
) -> ProverResult<DaWitness> {
    if da_refs.is_empty() {
        return Err(PaasError::PermanentFailure(
            "non-genesis batch has no DA refs".to_string(),
        ));
    }

    let mut sorted: Vec<&L1DaBlockRef> = da_refs.iter().collect();
    sorted.sort_by_key(|r| r.block.height());

    let mut blocks = Vec::with_capacity(sorted.len());
    let mut included_txs = Vec::new();
    for da_ref in sorted {
        let block_hash = da_ref.block.blkid().to_block_hash();
        let block = btc
            .get_block(&block_hash)
            .await
            .map_err(|e| PaasError::Storage(format!("get_block({block_hash}): {e}")))?;
        if block.txdata.is_empty() {
            return Err(PaasError::PermanentFailure(format!(
                "L1 block {block_hash} has no transactions"
            )));
        }
        let computed_wtxids_root = compute_wtxids_root(&block.txdata);
        if computed_wtxids_root != *da_ref.block.wtxids_root().as_ref() {
            let computed_wtxids_root = WtxidsRoot::from(Buf32::from(computed_wtxids_root));
            return Err(PaasError::PermanentFailure(format!(
                "L1 block {block_hash} wtxids_root mismatch: DA ref has {}, fetched block has {}",
                da_ref.block.wtxids_root(),
                computed_wtxids_root,
            )));
        }

        let mut txs = Vec::with_capacity(da_ref.txns.len());
        for (txid, wtxid) in &da_ref.txns {
            let pos = block
                .txdata
                .iter()
                .position(|tx| {
                    tx.compute_txid().to_byte_array() == txid.to_byte_array()
                        && tx.compute_wtxid().to_byte_array() == wtxid.to_byte_array()
                })
                .ok_or_else(|| {
                    PaasError::PermanentFailure(format!(
                        "DA tx {txid}/{wtxid} not found in L1 block {block_hash}"
                    ))
                })?;
            let proof = build_wtxid_inclusion_proof(&block.txdata, pos);
            let tx = block.txdata[pos].clone();
            txs.push(DaTxWitness::new(btc_serialize(&tx), proof));
            included_txs.push(tx);
        }

        blocks.push(DaBlockWitness::new(
            L1DaBlockInclusion::new(
                da_ref.block.height(),
                *da_ref.block.blkid().as_ref(),
                *da_ref.block.wtxids_root().as_ref(),
            ),
            txs,
        ));
    }

    let blob = reassemble_da_blob_from_txs(&included_txs)
        .map_err(|e| PaasError::PermanentFailure(format!("reassemble DA witness blob: {e}")))?;
    let unfiltered_state_diff =
        build_unfiltered_batch_state_diff(batch_block_hashes, state_diff_provider)?;
    let known_bytecodes =
        build_known_bytecode_witnesses(&blob, &unfiltered_state_diff, bytecode_store).await?;

    Ok(DaWitness::new_with_known_bytecodes(blocks, known_bytecodes))
}

fn build_unfiltered_batch_state_diff(
    block_hashes: &[Hash],
    state_diff_provider: &(impl StateDiffProvider + Sync + ?Sized),
) -> ProverResult<BatchStateDiff> {
    let mut builder = BatchBuilder::new();

    for block_hash in block_hashes {
        let b256 = B256::from(block_hash.0);
        let block_diff = state_diff_provider
            .get_state_diff_by_hash(b256)
            .map_err(|e| PaasError::Storage(format!("get_state_diff_by_hash({b256:?}): {e}")))?
            .ok_or_else(|| {
                PaasError::TransientFailure(format!(
                    "state diff missing for block {block_hash:?} while building DA witness"
                ))
            })?;
        builder.apply_block(&block_diff);
    }

    Ok(builder.build())
}

async fn build_known_bytecode_witnesses(
    blob: &alpen_ee_common::DaBlob,
    unfiltered_state_diff: &BatchStateDiff,
    bytecode_store: &impl AccessedStateStore,
) -> ProverResult<Vec<DaBytecodeWitness>> {
    let (mut known_bytecodes, unresolved) =
        known_bytecodes_from_unfiltered_diff(blob, unfiltered_state_diff);

    for code_hash in unresolved {
        let storage_key = Hash::from(code_hash.0);
        let bytecode = bytecode_store
            .get_bytecode(storage_key)
            .await
            .map_err(|e| PaasError::Storage(format!("get_bytecode({storage_key:?}): {e}")))?
            .ok_or_else(|| {
                PaasError::TransientFailure(format!(
                    "missing deduped bytecode {storage_key:?} while building DA witness"
                ))
            })?;
        known_bytecodes.push(DaBytecodeWitness::new(code_hash.0, bytecode));
    }

    Ok(known_bytecodes)
}

/// Decodes a `ChunkInput`'s transition bytes. `PermanentFailure` on malformed.
fn decode_chunk_transition(ci: &ChunkInput) -> ProverResult<ChunkTransition> {
    ci.try_decode_chunk_transition()
        .map_err(|e| PaasError::PermanentFailure(format!("decode chunk transition: {e:?}")))
}

/// Collect [`ChunkInput`]s for a batch by reading per-chunk receipts from
/// the shared paas `ReceiptStore` (the chunk prover writes them after
/// proving).
///
/// Returns `TransientFailure` if any chunk's receipt is not yet present
/// (paas will retry on tick); returns `PermanentFailure` if a stored
/// receipt fails to decode as a [`ChunkTransition`] (data corruption).
async fn collect_chunk_inputs_for_batch(
    batch_storage: &dyn BatchStorage,
    chunk_receipts: &dyn ReceiptStore,
    batch_id: BatchId,
) -> ProverResult<Vec<ChunkInput>> {
    let chunk_ids = batch_storage
        .get_batch_chunks(batch_id)
        .await
        .map_err(|e| PaasError::Storage(format!("get_batch_chunks({batch_id}): {e}")))?
        .ok_or_else(|| {
            PaasError::PermanentFailure(format!("no chunks set for batch {batch_id}"))
        })?;

    let mut chunks = Vec::with_capacity(chunk_ids.len());
    for chunk_id in chunk_ids {
        let key: Vec<u8> = ChunkTask(chunk_id).into();
        let receipt = chunk_receipts.get(&key)?.ok_or_else(|| {
            PaasError::TransientFailure(format!(
                "chunk receipt missing for {chunk_id:?} (batch {batch_id})"
            ))
        })?;
        let pubvals = receipt.receipt().public_values().as_bytes();
        let transition = ChunkTransition::from_ssz_bytes(pubvals).map_err(|e| {
            PaasError::PermanentFailure(format!("decode ChunkTransition for {chunk_id:?}: {e:?}"))
        })?;
        let proof_bytes = receipt.receipt().proof().as_bytes().to_vec();
        chunks.push(ChunkInput::new(transition, proof_bytes));
    }

    Ok(chunks)
}
