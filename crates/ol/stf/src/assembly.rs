//! Block assembly flows.
// TODO should this be in another crate?

use strata_identifiers::Buf32;
use strata_ledger_types::StateAccessor;
use strata_ol_chain_types_new::{
    OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLLog, OLTxSegment,
};

use crate::{
    context::BlockContext,
    errors::ExecResult,
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
        todo!()
    }
}

/// Performs execution using parts of a block on top of a state, producing
/// records of its output that we can use to complete a header for that drafted
/// block.
///
/// This closely aligns with `verify_block_classically`.
pub fn execute_block_inputs<S: StateAccessor>(
    state: &mut S,
    block_context: &BlockContext,
    block_exec_input: BlockExecInput<'_>,
) -> ExecResult<BlockExecOutputs> {
    // 0. Construct the block exec context for tracking verification state
    // across phases.
    // TODO

    // 1. If it's the first block of the epoch, call process_epoch_initial.
    // TODO

    // 2. Call process_block_tx_segment for every block as usual.
    // TODO

    // 3. Compute the state root and remember it.
    // TODO

    // 4. If it's the last block of an epoch, then call process_block_manifests, and compute the
    //    final state root and remember it.
    // TODO

    todo!()
}

/// Parts of a block we're trying to construct.
#[derive(Clone, Debug)]
pub struct BlockComponents {
    tx_segment: OLTxSegment,
    manifest_container: Option<OLL1ManifestContainer>,
}

impl BlockComponents {
    fn new(tx_segment: OLTxSegment, manifest_container: Option<OLL1ManifestContainer>) -> Self {
        Self {
            tx_segment,
            manifest_container,
        }
    }

    pub fn tx_segment(&self) -> &OLTxSegment {
        &self.tx_segment
    }

    pub fn manifest_container(&self) -> Option<&OLL1ManifestContainer> {
        self.manifest_container.as_ref()
    }

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
    block_context: &BlockContext,
    block_components: BlockComponents,
) -> ExecResult<CompletedBlock> {
    // 1. Execute the block.
    // TODO

    // 2. Take the inputs and outputs and compute the commitments for the header.
    // TODO

    // 3. Assemble the final completed block.
    // TODO

    todo!()
}
