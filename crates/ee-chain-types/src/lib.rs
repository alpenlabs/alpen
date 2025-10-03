//! Strata common EE chain types.
//!
//! This is primarily at the boundary between the internal EE account state and
//! the execution env chain.  These are not generally involved in the
//! orchestration layer protocol.
// @jose: fineeeee

mod block;

pub use block::{
    BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockNotpackage, OutputTransfer,
    SubjectDepositData,
};
