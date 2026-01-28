//! Snark account runtime traits.

use std::error::Error;

use ssz::{Decode, Encode};
use strata_acct_types::Hash;
use strata_codec::Codec;

use crate::{InputMessage, errors::ProgramResult};

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

    /// Recognized messages.  If parsing returns an error then we pass
    /// [`InputMessage::Unknown`] to `verify_coinput and `process_message`.
    type Msg: IAcctMsg;

    /// Update extra data.
    type ExtraData: IExtraData;

    /// Error type.
    type Error: Error;

    /// Starts an update, making whatever initial changes there are before
    /// handling messages.
    fn start_update(
        &self,
        _state: &mut Self::State,
        _extra_data: &Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        Ok(())
    }

    /// Processes a verified message, updating the state.
    fn process_message(
        &self,
        _state: &mut Self::State,
        _msg: InputMessage<Self::Msg>,
        _extra_data: &Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        Ok(())
    }

    /// Applies any final state changes after processing messages but before
    /// performing verification finalization checks.
    fn pre_finalize_state(
        &self,
        _state: &mut Self::State,
        _extra_data: &Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        Ok(())
    }

    /// Finalizes the state after performing final checks.
    fn finalize_state(
        &self,
        _state: &mut Self::State,
        _extra_data: Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        Ok(())
    }
}

/// Trait that describes the "verification" procedures of a snark account
/// program, used in contexts like in an update proof.
pub trait SnarkAccountProgramVerification: SnarkAccountProgram {
    /// Temporary state that can be modified while processing coinputs but is
    /// not persisted or accessible when modifying the state.
    ///
    /// This is a GAT parameterized by lifetime to allow storing references,
    /// especially ones taken from [`VInput`].
    type VState<'a>;

    /// Private input data required for verification.
    ///
    /// This is passed by value to [`Self::start_verification`] and its contents
    /// can be moved into [`Self::VState`] for use during verification. This
    /// allows the program to receive verification-specific data (e.g., chain
    /// segments, pre-state) without storing it in the program struct.
    ///
    /// This is a GAT parameterized by lifetime to allow storing references.
    type VInput<'a>;

    /// Creates the verification context.  This is called before [`start_update`].
    ///
    /// The `vinput` parameter provides private input data needed for verification.
    /// It is passed by value so that its contents (typically references) can be
    /// moved into the returned `VState`.
    fn start_verification<'a>(
        &self,
        state: &Self::State,
        extra_data: &Self::ExtraData,
        vinput: Self::VInput<'a>,
    ) -> ProgramResult<Self::VState<'a>, Self::Error>;

    /// Verifies a coinput for a message against the current state.
    ///
    /// This may parse the coinput dependent on the message, and may error if
    /// the coinput is malformed/invalid and this has not been handled
    /// appropriately by the update producer.
    fn verify_coinput<'a>(
        &self,
        _state: &Self::State,
        _vstate: &mut Self::VState<'a>,
        _msg: &InputMessage<Self::Msg>,
        _coinput: &[u8],
        _extra_data: &Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        // By default apply no constraints.
        Ok(())
    }

    /// Performs any final verification checks, consuming the vstate.
    fn finalize_verification<'a>(
        &self,
        _state: &Self::State,
        _vstate: Self::VState<'a>,
        _extra_data: &Self::ExtraData,
    ) -> ProgramResult<(), Self::Error> {
        // By default apply no constraints.
        Ok(())
    }
}

/// Trait describing the program's account state.
pub trait IInnerState: Clone + Encode + Decode + 'static {
    /// Computes a commitment to the inner state.
    ///
    /// The return value of this function corresponds to the `inner_state` field
    /// in the snark account state in the orchestration layer ledger.
    fn compute_state_root(&self) -> Hash;
}

/// Trait describing account messages recognized by the program.
///
/// This should probably be implemented on an enum.
pub trait IAcctMsg: Clone + 'static {
    /// Error type returned when parsing fails.
    type ParseError;

    /// Attempts to parse a message from a raw buffer.
    ///
    /// Returns `Ok(msg)` if parsing succeeds, `Err(e)` if the message format
    /// is invalid or unrecognized.
    fn try_parse(buf: &[u8]) -> Result<Self, Self::ParseError>;
}

/// Trait describing the extra data processed by the snark account.
pub trait IExtraData: Clone + Codec + 'static {
    // TODO
}
