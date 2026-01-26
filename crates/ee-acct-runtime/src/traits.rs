//! Snark account runtime traits.

use strata_acct_types::Hash;
use strata_codec::Codec;

use crate::errors::ProgramResult;

/// Describes a snark account program in terms of its state, the messages it
/// receives, and the kinds of checks that get performed secretly as part of the
/// process of proving an update.
///
/// These functions are structured in such a way that an implementor can only
/// ever make modifications to the committed account state using data that is
/// ensured to be durably stored, but we can have some rich state that we can
/// use to perform checks across the state.
pub trait SnarkAccountProgram {
    /// Account inner state.
    type State: IInnerState;

    /// Temporary state that can be modified while processing coinputs but is
    /// not persisted or accessible when modifying the state.
    type VState;

    /// Recognized messages.
    type Msg: IAcctMsg;

    /// Update extra data.
    type ExtraData: IExtraData;

    /// Starts an update, also producing a verification state we use while
    /// processing coinputs.
    ///
    /// The [`Self::VState`] result may be simply discarded if we're not in a
    /// context where we can verify coinputs using it.
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
    /// performing verification finalization checks.
    fn pre_finalize_state(
        &self,
        state: &mut Self::State,
        extra_data: &Self::ExtraData,
    ) -> ProgramResult<()>;

    /// Performs any final verification checks, consuming the vstate.
    fn finalize_verification(
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

/// Trait describing the program's account state.
pub trait IInnerState: Clone + Codec + 'static {
    /// Computes a commitment to the inner state.
    ///
    /// The return value of this function corresponds to the `inner_state` field
    /// in the snark account state in the orchestration layer ledger.
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
