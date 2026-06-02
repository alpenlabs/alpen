use strata_db_types::errors::DbError;
use strata_identifiers::Epoch;
use strata_predicate::PredicateError;
use strata_primitives::{
    l1::{L1BlockCommitment, L1BlockId},
    l2::L2BlockId,
    prelude::*,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing client state index {0}")]
    MissingClientState(L1BlockCommitment),

    #[error("L2 blkid {0:?} missing from database")]
    MissingL2Block(L2BlockId),

    #[error("missing OL block {0}")]
    MissingOLBlock(OLBlockId),

    #[error("missing OL state for block {0}")]
    MissingOLState(OLBlockCommitment),

    #[error("L1 blkid {0:?} missing from database")]
    MissingL1Block(L1BlockId),

    #[error("L1Tx missing from database")]
    MissingL1Tx,

    #[error("L1 block {0} missing from database")]
    MissingL1BlockHeight(u64),

    #[error("missing expected consensus writes at {0}")]
    MissingConsensusWrites(u64),

    #[error("missing expected chainstate for blockidx {0}")]
    MissingIdxChainstate(u64),

    #[error("missing expected chainstate for block {0:?}")]
    MissingBlockChainstate(L2BlockId),

    #[error("OL block {0:?} missing signature")]
    MissingBlockSignature(OLBlockId),

    #[error("OL block {0:?} signature invalid")]
    InvalidBlockSignature(OLBlockId),

    #[error("unexpected genesis OL block {0:?}")]
    UnexpectedGenesisBlock(OLBlockId),

    /// Reorg has a non-empty down branch but its pivot is not before the current tip.
    #[error("OL reorg pivot {0} not before current tip {1}")]
    InvalidOLReorgPivot(OLBlockCommitment, OLBlockCommitment),

    /// Reorg has no down branch, but its pivot is not the current tip.
    #[error("invalid OL reorg pivot mismatch with empty down (expected {0}, got {1})")]
    InvalidOLReorgEmptyDownPivot(OLBlockCommitment, OLBlockCommitment),

    /// FCM attempted to apply a block whose header parent is not the current tip.
    #[error("OL apply block parent mismatch for block {0} (expected {1}, got {2})")]
    OLApplyBlockParentMismatch(OLBlockCommitment, OLBlockCommitment, OLBlockId),

    /// FCM finished applying a tip update but did not land on the expected block ID.
    #[error("OL apply tip mismatch (expected {0}, got {1})")]
    OLApplyTipMismatch(OLBlockCommitment, OLBlockCommitment),

    #[error("csm dropped")]
    CsmDropped,

    #[error("tried to process competing block for height {0} (have {0}, given {1})")]
    CompetingBlock(u64, L1BlockId, L1BlockId),

    #[error("failed creating genesis chain state: {0}")]
    GenesisFailed(String),

    #[error("not yet implemented")]
    Unimplemented,

    #[error("deserializing failed")]
    Deserialization,

    #[error("deserializing tx failed for index: {0}")]
    TxDeserializationFailed(u64),

    #[error("chain is not active yet")]
    ChainInactive,

    #[error("checkpoint invalid: {0}")]
    InvalidCheckpoint(#[from] CheckpointError),

    #[error("tried to finalize epoch {0:?} but epoch {1:?} is already final")]
    FinalizeOldEpoch(EpochCommitment, EpochCommitment),

    #[error("stateroot mismatch")]
    StaterootMismatch,

    #[error("chaintip: {0}")]
    ChainTip(#[from] ChainTipError),

    #[error("db: {0}")]
    Db(#[from] DbError),

    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Error)]
pub enum ChainTipError {
    #[error("tried to attach blkid {0:?} but missing parent blkid {1:?}")]
    AttachMissingParent(L2BlockId, L2BlockId),

    #[error("tried to finalize unknown block {0:?}")]
    MissingBlock(L2BlockId),

    /// This should only happen with malformed blocks.
    #[error("child slot {0} was leq declared parent slot {1}")]
    ChildBeforeParent(u64, u64),
}

#[derive(Debug, Error)]
pub enum CheckpointError {
    /// Constructed when we don't have a previous checkpoint so we're expecting
    /// one for genesis.
    #[error("skipped genesis epoch")]
    SkippedGenesis,

    #[error("checkpoint is epoch {0} on top of previous checkpoint {1}")]
    Sequencing(Epoch, Epoch),

    #[error("L1 state transition mismatch")]
    MismatchL1State,

    #[error("L2 state transition mismatch")]
    MismatchL2State,

    #[error("signature is invalid")]
    InvalidSignature,

    #[error("transition is malformed")]
    MalformedTransition,

    #[error("transition doesn't match the expected")]
    TransitionMismatch,

    #[error("proof validation: {0}")]
    Proof(#[from] PredicateError),
}
