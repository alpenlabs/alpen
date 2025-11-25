//! Block assembly flows.

use strata_asm_common::AsmManifest;
use strata_identifiers::Buf32;
use strata_ledger_types::StateAccessor;
use strata_merkle::{BinaryMerkleTree, Sha256Hasher};
use strata_ol_chain_types_new::{
    BlockFlags, OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLL1Update, OLLog,
    OLTransaction, OLTxSegment, TransactionAttachment, TransactionPayload,
};

use crate::{
    chain_processing,
    context::{BasicExecContext, BlockContext, TxExecContext},
    errors::ExecResult,
    manifest_processing,
    output::{ExecOutputBuffer, OutputCtx},
    transaction_processing,
    verification::{BlockExecInput, BlockPostStateCommitments},
};

/// Block execution outputs.
///
/// These can be used to construct a final block.
#[derive(Clone, Debug)]
pub struct BlockExecOutputs {
    post_state_roots: BlockPostStateCommitments,
    logs: Vec<OLLog>,
}

impl BlockExecOutputs {
    fn new(post_state_roots: BlockPostStateCommitments, logs: Vec<OLLog>) -> Self {
        Self {
            post_state_roots,
            logs,
        }
    }

    pub fn post_state_roots(&self) -> &BlockPostStateCommitments {
        &self.post_state_roots
    }

    pub fn header_post_state_root(&self) -> &Buf32 {
        self.post_state_roots.header_state_root()
    }

    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    /// Computes the block's logs root from the log.
    pub fn compute_block_logs_root(&self) -> Buf32 {
        // This is just a simple binary merkle tree.
        if self.logs.is_empty() {
            // Empty tree has null root
            return Buf32::zero();
        }

        // Hash each log entry to create leaf nodes
        let mut leaf_hashes: Vec<[u8; 32]> = self
            .logs
            .iter()
            .map(|log| log.compute_hash_commitment().0)
            .collect();

        // BinaryMerkleTree requires power of two leaves, so pad if necessary
        let next_power_of_two = leaf_hashes.len().next_power_of_two();
        while leaf_hashes.len() < next_power_of_two {
            // Pad with zero hashes
            leaf_hashes.push([0u8; 32]);
        }

        // Build the merkle tree using Sha256Hasher
        let tree = BinaryMerkleTree::from_leaves::<Sha256Hasher>(leaf_hashes)
            .expect("power of two leaves should always succeed");

        Buf32(*tree.root())
    }
}

/// Performs execution using parts of a block on top of a state, producing
/// records of its output that we can use to complete a header for that drafted
/// block.
///
/// This closely aligns with `verify_block_classically`.
pub fn execute_block_inputs<S: StateAccessor>(
    state: &mut S,
    block_context: BlockContext<'_>,
    block_exec_input: BlockExecInput<'_>,
) -> ExecResult<BlockExecOutputs> {
    // 0. Construct the block exec context for tracking verification state
    // across phases.
    let output = ExecOutputBuffer::new_empty();

    // 1. If it's the first block of the epoch, call process_epoch_initial.
    if block_context.is_epoch_initial() {
        let init_ctx = block_context.to_epoch_initial_context();
        chain_processing::process_epoch_initial(state, &init_ctx)?;
    }

    // 2. Process the slot start for every block.
    chain_processing::process_block_start(state, &block_context)?;

    // 3. Call process_block_tx_segment for every block as usual.
    let basic_ctx = BasicExecContext::new(*block_context.block_info(), &output);
    let tx_ctx = TxExecContext::new(&basic_ctx, block_context.parent_header());
    transaction_processing::process_block_tx_segment(
        state,
        block_exec_input.tx_segment(),
        &tx_ctx,
    )?;

    // 4. Compute the state root and remember it.
    let pre_manifest_state_root = state.compute_state_root()?;

    // 5. If it's the last block of an epoch, then call process_block_manifests,
    // and compute the final state root and remember it.
    //
    // Then we use this to figure out what our state commitments should be.
    let post_state_roots = if let Some(manifest_container) = block_exec_input.manifest_container() {
        // Terminal block, with manifests.
        let term_ctx = tx_ctx.basic_context();
        manifest_processing::process_block_manifests(state, manifest_container, term_ctx)?;

        // Then finally extract the stuff.
        let final_state_root = state.compute_state_root()?;
        BlockPostStateCommitments::Terminal(pre_manifest_state_root, final_state_root)
    } else {
        // Regular non-terminal block.
        BlockPostStateCommitments::Common(pre_manifest_state_root)
    };

    // Extract logs from the execution context and construct the final output.
    let logs = output.into_logs();
    Ok(BlockExecOutputs::new(post_state_roots, logs))
}

/// Parts of a block we're trying to construct.
#[derive(Clone, Debug)]
pub struct BlockComponents {
    tx_segment: OLTxSegment,
    manifest_container: Option<OLL1ManifestContainer>,
}

impl BlockComponents {
    pub fn new(tx_segment: OLTxSegment, manifest_container: Option<OLL1ManifestContainer>) -> Self {
        Self {
            tx_segment,
            manifest_container,
        }
    }

    /// Create new empty block components.
    pub fn new_empty() -> Self {
        Self {
            tx_segment: OLTxSegment::new(Vec::new()),
            manifest_container: None,
        }
    }

    /// Create block components with the given transaction payloads.
    pub fn new_txs(payloads: Vec<TransactionPayload>) -> Self {
        let txs = payloads
            .into_iter()
            .map(|p| OLTransaction::new(TransactionAttachment::default(), p))
            .collect();
        Self {
            tx_segment: OLTxSegment::new(txs),
            manifest_container: None,
        }
    }

    /// Create terminal block components from manifests.
    pub fn new_manifests(manifests: Vec<AsmManifest>) -> Self {
        Self {
            tx_segment: OLTxSegment::new(Vec::new()),
            manifest_container: Some(OLL1ManifestContainer::new(manifests)),
        }
    }

    pub fn tx_segment(&self) -> &OLTxSegment {
        &self.tx_segment
    }

    pub fn manifest_container(&self) -> Option<&OLL1ManifestContainer> {
        self.manifest_container.as_ref()
    }

    /// Creates a [`BlockExecInput`] which is more or less really just a
    /// borrowed version of this type.
    pub fn to_exec_input(&self) -> BlockExecInput<'_> {
        BlockExecInput::new(&self.tx_segment, self.manifest_container.as_ref())
    }
}

/// A block that has a completed header and body, but does not have a signature.
#[derive(Clone, Debug)]
pub struct CompletedBlock {
    header: OLBlockHeader,
    body: OLBlockBody,
}

impl CompletedBlock {
    fn new(header: OLBlockHeader, body: OLBlockBody) -> Self {
        Self { header, body }
    }

    pub fn header(&self) -> &OLBlockHeader {
        &self.header
    }

    pub fn body(&self) -> &OLBlockBody {
        &self.body
    }
}

/// Given components of a block, executes it and uses it to construct the
/// components of a block that can be signed.
pub fn execute_and_complete_block<S: StateAccessor>(
    state: &mut S,
    block_context: BlockContext<'_>,
    block_components: BlockComponents,
) -> ExecResult<CompletedBlock> {
    // 1. First just execute the block with the inputs.
    let block_exec_input = block_components.to_exec_input();
    let exec_outputs = execute_block_inputs(state, block_context, block_exec_input)?;

    // 2. Take the inputs and outputs and compute the commitments for the header.

    // Compute the logs root from the execution outputs.
    let logs_root = exec_outputs.compute_block_logs_root();

    // Get the state root from the execution outputs.
    let state_root = *exec_outputs.header_post_state_root();

    // Compute the parent block ID.
    let parent_blkid = block_context.compute_parent_blkid();

    // Construct the block body.
    let mut body = OLBlockBody::new_common(block_components.tx_segment);

    // If this is a terminal block with manifests, create the L1 update.
    if let Some(manifest_container) = block_components.manifest_container {
        if let Some(preseal_root) = exec_outputs.post_state_roots().preseal_state_root() {
            let l1_update = OLL1Update::new(*preseal_root, manifest_container);
            body.set_l1_update(l1_update);
        }
    }

    // Compute the body root using the hash commitment method.
    let body_root = body.compute_hash_commitment();
    let mut flags = BlockFlags::zero();
    flags.set_is_terminal(body.is_body_terminal());

    // 3. Assemble the final completed block.
    let header = OLBlockHeader::new(
        block_context.timestamp(),
        flags,
        block_context.slot(),
        block_context.epoch(),
        parent_blkid,
        body_root,
        state_root,
        logs_root,
    );

    Ok(CompletedBlock::new(header, body))
}
