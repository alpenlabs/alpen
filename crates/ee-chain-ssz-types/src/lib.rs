//! SSZ type definitions for execution environment chain types.
//!
//! This crate contains pure SSZ type definitions that match the pythonic schema
//! in `schemas/ee-chain-types.ssz`. These types are included by `strata-ee-chain-types`
//! which adds business logic.

pub mod block;

pub use block::{
    BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockNotpackage,
    MAX_OUTPUT_MESSAGES_PER_BLOCK, MAX_OUTPUT_TRANSFERS_PER_BLOCK, MAX_SUBJECT_DEPOSITS_PER_BLOCK,
    OutputTransfer, SubjectDepositData,
};
