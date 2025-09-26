//! Verification state accumulator.

use strata_acct_types::BitcoinAmount;
use strata_ee_acct_types::{CommitBlockData, CommitChainSegment, CommitMsgData, EeAccountState};
use strata_ee_chain_types::BlockOutputs;
use strata_snark_acct_types::{OutputMessage, OutputTransfer, UpdateOutputs};

type Hash = [u8; 32];

/// State tracker that accumulates changes that we need to make checks about
/// later on in update processing.
#[derive(Debug)]
pub struct UpdateVerificationState {
    // balance bookkeeping as additional checks to avoid overdraw
    orig_tracked_balance: BitcoinAmount,
    total_val_sent: BitcoinAmount,
    total_val_recv: BitcoinAmount,

    // commits to check
    pending_commits: Vec<CommitMsgData>,

    // recorded outputs we'll check later
    accumulated_outputs: UpdateOutputs,

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
            total_val_sent: 0.into(),
            total_val_recv: 0.into(),
            pending_commits: Vec::new(),
            accumulated_outputs: UpdateOutputs::new_empty(),
            l1_da_blob_hashes: Vec::new(),
        }
    }

    pub fn accumulated_outputs(&self) -> &UpdateOutputs {
        &self.accumulated_outputs
    }

    /// Appends a notpackage block's outputs into the pending outputs being
    /// built internally.  This way we can compare it against the update op data
    /// later.
    pub fn merge_block_outputs(&mut self, outputs: &BlockOutputs) {
        // Just merge the entries into the buffer.  This is a little more
        // complicated than it really is because we have to convert between two
        // sets of similar types that are separately defined to avoid semantic
        // confusion because they do refer to different concepts.
        self.accumulated_outputs.transfers_mut().extend(
            outputs
                .output_transfers()
                .iter()
                .map(|e| OutputTransfer::new(e.dest(), e.value())),
        );

        self.accumulated_outputs.messages_mut().extend(
            outputs
                .output_messages()
                .iter()
                .map(|e| OutputMessage::new(e.dest(), e.payload().clone())),
        );

        // Annoying thing to do checked summation.
        let sent_amts_iter = [self.total_val_sent]
            .into_iter()
            .chain(outputs.output_transfers().iter().map(|e| e.value()))
            .chain(
                outputs
                    .output_messages()
                    .iter()
                    .map(|e| e.payload().value()),
            );

        self.total_val_sent = BitcoinAmount::sum(sent_amts_iter);
    }
}
