//! Verification state accumulator.

use strata_ee_acct_types::EeAccountState;
use strata_snark_acct_types::{OutputMessage, OutputTransfer};

type Hash = [u8; 32];

/// State tracker that accumulates changes that we need to make checks about
/// later on in update processing.
#[derive(Debug)]
pub struct UpdateVerificationState {
    // balance bookkeeping as additional checks to avoid overdraw
    orig_tracked_balance: u64,
    total_val_sent: u64,
    total_val_recv: u64,

    // recorded outputs we'll check later
    output_transfers: Vec<OutputTransfer>,
    output_messages: Vec<OutputMessage>,

    // Recorded DA.
    l1_da_blob_hashes: Vec<Hash>,
}

impl UpdateVerificationState {
    /// Constructs a verification state using the account's initial state as a
    /// reference.
    ///
    /// We don't take ownership of it, because that makes the types less clean
    /// to work with later on and breaks our use of the type system to enforce
    /// correctness about not updating the state with private information.
    pub fn new_from_state(state: &EeAccountState) -> Self {
        Self {
            orig_tracked_balance: state.tracked_balance(),
            total_val_sent: 0,
            total_val_recv: 0,
            output_transfers: Vec::new(),
            output_messages: Vec::new(),
            l1_da_blob_hashes: Vec::new(),
        }
    }
}
