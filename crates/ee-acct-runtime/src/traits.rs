//! Snark account runtime traits.

use strata_acct_types::Hash;
use strata_codec::{Codec, CodecError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProgramError {
    #[error("invalid coinput")]
    InvalidCoinput,

    #[error("malformed coinput")]
    MalformedCoinput,

    #[error("obligations unsatisfied after update finished processing")]
    UnsatisfiedObligations,
}

pub type ProgramResult<T> = Result<T, ProgramError>;

pub trait SnarkAccountProgram {
    /// Account inner state.
    type State: IInnerState;

    /// Temporary state that can be modified while processing coinputs but isno .
    type VState;

    /// Recognized messages.
    type Msg: IAcctMsg;

    /// Update extra data.
    type ExtraData: IExtraData;

    /// Starts an update, also producing a verification state we use while
    /// processing coinputs.
    ///
    /// The result is discarded if we're not in a proof.
    fn start_update(
        &self,
        state: &mut Self::State,
        extra_data: &Self::ExtraData,
    ) -> ProgramResult<Self::VState>;

    /// Verifies a coinput for a message against the current state.
    ///
    /// This may parse the coinput dependent on the message, and may error if
    /// the coinput is malformed/invalid and this has not been handled
    /// appropriately by the update producer.
    fn verify_coinput(
        &self,
        state: &Self::State,
        vstate: &mut Self::VState,
        msg: &Self::Msg,
        coinput: &[u8],
        extra_data: &Self::ExtraData,
    ) -> ProgramResult<()>;

    /// Processes a verified message, updating the state.
    fn process_message(
        &self,
        state: &mut Self::State,
        msg: Self::Msg,
        extra_data: &Self::ExtraData,
    ) -> ProgramResult<()>;

    /// Applies any final state changes after processing messages but before
    /// performing finalization checks.
    fn pre_finalize_state(
        &self,
        state: &mut Self::State,
        extra_data: &Self::ExtraData,
    ) -> ProgramResult<()>;

    /// Performs any final checks against the verification state.
    fn finalize_update(
        &self,
        state: &Self::State,
        vstate: Self::VState,
        extra_data: &Self::ExtraData,
    ) -> ProgramResult<()>;

    /// Finalizes the state after performing final checks.
    fn finalize_state(
        &self,
        state: &mut Self::State,
        extra_data: Self::ExtraData,
    ) -> ProgramResult<()>;
}

/// Trait describing the program state.
pub trait IInnerState: Clone + Codec + 'static {
    /// Computes a commitment to the inner state.
    fn compute_state_root(&self) -> Hash;
}

/// Trait describing account messages recognized by the program.
///
/// This should probably be implemented on an enum.
pub trait IAcctMsg: Clone + Codec + 'static {
    // TODO
}

/// Trait describing the extra data processed by the snark account.
pub trait IExtraData: Clone + Codec + 'static {
    // TODO
}
