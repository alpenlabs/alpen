//! Types relating to snark accounts and the snark account proof interface.

mod accumulators;
mod messages;
mod outputs;
mod proof_interface;
mod state;
mod update;

// Include generated SSZ types from build.rs output
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use proof_interface::UpdateProofPubParams;
pub use ssz_generated::ssz::{
    accumulators::{AccumulatorClaim, AccumulatorClaimRef, MmrEntryProof, MmrEntryProofRef},
    messages::{MessageEntry, MessageEntryProof, MessageEntryRef},
    outputs::{
        MAX_MESSAGES, MAX_TRANSFERS, OutputMessage, OutputTransfer, UpdateOutputs, UpdateOutputsRef,
    },
    state::{MAX_VK_BYTES, ProofState, ProofStateRef, SnarkAccountState, SnarkAccountStateRef},
    update::{
        LedgerRefs, LedgerRefsRef, MAX_EXTRA_DATA_BYTES, MAX_LEDGER_REFS, MAX_PROCESSED_MESSAGES,
        MAX_UPDATE_PROOF_BYTES, SnarkAccountUpdate, SnarkAccountUpdateContainer,
        SnarkAccountUpdateContainerRef, SnarkAccountUpdateRef, UpdateAccumulatorProofs,
        UpdateAccumulatorProofsRef, UpdateInputData, UpdateInputDataRef, UpdateOperationData,
        UpdateOperationDataRef, UpdateStateData, UpdateStateDataRef,
    },
};
