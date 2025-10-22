// SSZ type definitions for execution environment block structures.
// Types defined here match the pythonic schema in `schemas/ee-chain-types.ssz`.

use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, SentMessage, SubjectId};
use tree_hash_derive::TreeHash as TreeHashDerive;

// Constants for list bounds (from schema)
pub const MAX_SUBJECT_DEPOSITS_PER_BLOCK: usize = 1 << 16; // 65536
pub const MAX_OUTPUT_TRANSFERS_PER_BLOCK: usize = 1 << 16; // 65536
pub const MAX_OUTPUT_MESSAGES_PER_BLOCK: usize = 1 << 16; // 65536

/// Variable-length list for subject deposits
type SubjectDepositList = VariableList<SubjectDepositData, MAX_SUBJECT_DEPOSITS_PER_BLOCK>;

/// Variable-length list for output transfers
type OutputTransferList = VariableList<OutputTransfer, MAX_OUTPUT_TRANSFERS_PER_BLOCK>;

/// Variable-length list for output messages
type OutputMessageList = VariableList<SentMessage, MAX_OUTPUT_MESSAGES_PER_BLOCK>;

/// Container for an execution block that signals additional data with it.
// TODO better name, using an intentionally bad one for now
/// Schema: class ExecBlockNotpackage(Container)
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct ExecBlockNotpackage {
    /// Commitment to the block itself.
    commitment: ExecBlockCommitment,

    /// Inputs processed in the block.
    inputs: BlockInputs,

    /// Outputs produced in the block.
    outputs: BlockOutputs,
}

/// Commitment to a particular execution block, in multiple ways.
// should this contain parent and index information?
/// Schema: class ExecBlockCommitment(Container)
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Encode, Decode, TreeHashDerive)]
pub struct ExecBlockCommitment {
    /// Block ID as interpreted by the execution environment, probably a hash of
    /// a block header.
    ///
    /// This is so that the proofs are able to cheaply reason about the chain,
    /// using its native concepts.
    ///
    /// We can't *just* use `raw_block_encoded_hash`, because we would have to
    /// include the full block in the proof, and that doesn't even give us
    /// parent linkages.
    exec_blkid: Hash,

    /// Hash of the encoded block.
    ///
    /// This is so that we can know if we have the right block without knowing
    /// how to parse it.
    ///
    /// We can't *just* use `exec_blkid`, because we might not be in a context
    /// where we know how to parse a block in order to hash it.
    raw_block_encoded_hash: Hash,
}

/// Inputs from the OL to the EE processed in a single EE block.
/// Schema: class BlockInputs(Container)
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct BlockInputs {
    subject_deposits: SubjectDepositList,
}

/// Describes data for a simple deposit to a subject within an EE.
///
/// This is used for deposits from L1, but can encompass any "blind" transfer to
/// a subject (which doesn't allow it to autonomously respond to the deposit or
/// know where the sender was).
/// Schema: class SubjectDepositData(Container)
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct SubjectDepositData {
    dest: SubjectId,
    value: BitcoinAmount,
}

/// Outputs from an EE to the OL produced in a single EE block.
/// Schema: class BlockOutputs(Container)
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct BlockOutputs {
    output_transfers: OutputTransferList,
    output_messages: OutputMessageList,
}

/// Transfer output from EE to OL
/// Schema: class OutputTransfer(Container)
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct OutputTransfer {
    /// Destination orchestration layer account ID.
    dest: AccountId,

    /// Native asset value sent (satoshis).
    value: BitcoinAmount,
}
