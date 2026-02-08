//! Generic update processing for snark account programs.
//!
//! This module provides generic implementations of the two code paths for
//! processing snark account updates:
//!
//! - [`verify_and_apply_update`]: Full verification path used within SNARK proofs.
//! - [`apply_update_unconditionally`]: Reconstruction path used outside proofs after verification,
//!   to reconstruct state from DA.
//!
//! These functions are generic over the [`SnarkAccountProgram`] trait (and
//! [`SnarkAccountProgramVerification`] for the verification path), allowing
//! different account types to reuse the same processing logic.

use crate::{
    InputMessage,
    errors::{ProgramError, ProgramResult},
    traits::{SnarkAccountProgram, SnarkAccountProgramVerification},
};

/// Verifies and applies an update using the [`SnarkAccountProgramVerification`]
/// trait.
///
/// This is the full verification path used within SNARK proofs. It processes
/// each message with both coinput verification and state updates.
///
/// Messages are wrapped in [`InputMessage`] - can be `Valid` or `Unknown`.
/// ALL messages (including `Unknown`) are passed to the program, which decides
/// how to handle them.
///
/// # Type Parameters
///
/// - `P`: The snark account program implementation (must implement verification).
/// - `M`: Iterator over input messages.
/// - `C`: Iterator over coinput byte slices.
// TODO refactor this signature to operate less on already-decoded types and do
// more of the decoding itself
pub fn verify_and_apply_update<'a, 'c, P, M, C>(
    program: &'a P,
    state: &mut P::State,
    messages: M,
    coinputs: C,
    extra_data: P::ExtraData,
    vinput: P::VInput<'a>,
) -> ProgramResult<(), P::Error>
where
    P: SnarkAccountProgramVerification + 'a,
    M: IntoIterator<Item = InputMessage<P::Msg>>,
    C: IntoIterator<Item = &'c [u8]>,
{
    // 1. Start verification context with private input (moved).
    let mut vstate = program.start_verification(state, &extra_data, vinput)?;

    // 2. Start update.
    program.start_update(state, &extra_data)?;

    // 3. Process messages with verification.
    // ALL messages (Valid and Unknown) are passed to program.
    let mut coinp_iter = coinputs.into_iter().fuse();
    let mut msg_count = 0usize;
    for (idx, msg) in messages.into_iter().enumerate() {
        msg_count += 1;
        let coinp = coinp_iter.next().unwrap_or(&[]);

        program
            .verify_coinput(state, &mut vstate, &msg, coinp, &extra_data)
            .map_err(|e| e.at_msg(idx))?;

        program
            .process_message(state, msg, &extra_data)
            .map_err(|e| e.at_msg(idx))?;
    }

    // Check for extra coinputs that weren't consumed.
    let extra_coinputs = coinp_iter.count();
    if extra_coinputs > 0 {
        return Err(ProgramError::MismatchedCoinputCount {
            expected: msg_count,
            actual: msg_count + extra_coinputs,
        });
    }

    // 4. Pre-finalize state.
    program.pre_finalize_state(state, &extra_data)?;

    // 5. Finalize verification (consumes vstate).
    program.finalize_verification(state, vstate, &extra_data)?;

    // 6. Finalize state.
    program.finalize_state(state, extra_data)?;

    Ok(())
}

/// Applies an update unconditionally without verification.
///
/// This is used outside the proof, after verifying the proof, to reconstruct
/// the actual state from DA. It skips coinput verification and the
/// `finalize_verification` step.
///
/// Messages are wrapped in [`InputMessage`] - can be `Valid` or `Unknown`.
///
/// # Type Parameters
///
/// - `P`: The snark account program implementation.
/// - `M`: Iterator over input messages.
// TODO refactor this like the above function to do less of the decoding itself
pub fn apply_update_unconditionally<P, M>(
    program: &P,
    state: &mut P::State,
    messages: M,
    extra_data: P::ExtraData,
) -> ProgramResult<(), P::Error>
where
    P: SnarkAccountProgram,
    M: IntoIterator<Item = InputMessage<P::Msg>>,
{
    // 1. Start update.
    program.start_update(state, &extra_data)?;

    // 2. Process messages without verification.
    for (idx, msg) in messages.into_iter().enumerate() {
        program
            .process_message(state, msg, &extra_data)
            .map_err(|e| e.at_msg(idx))?;
    }

    // 3. Pre-finalize state.
    program.pre_finalize_state(state, &extra_data)?;

    // (4. Skip finalize_verification.)

    // 5. Finalize state.
    program.finalize_state(state, extra_data)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use ssz_derive::{Decode, Encode};
    use strata_acct_types::{AccountId, BitcoinAmount, Hash};
    use strata_codec::impl_type_flat_struct;

    use super::*;
    use crate::{
        MsgMeta,
        traits::{IAcctMsg, IExtraData, IInnerState},
    };

    // Simple test types for the generic processing functions.

    #[derive(Clone, Debug, Default, Encode, Decode)]
    struct TestState {
        value: u64,
    }

    impl IInnerState for TestState {
        fn compute_state_root(&self) -> Hash {
            Hash::default()
        }
    }

    impl_type_flat_struct! {
        #[derive(Clone, Debug)]
        struct TestMsg {
            delta: u64,
        }
    }

    impl IAcctMsg for TestMsg {
        type ParseError = strata_codec::CodecError;

        fn try_parse(buf: &[u8]) -> Result<Self, Self::ParseError> {
            strata_codec::decode_buf_exact(buf)
        }
    }

    impl_type_flat_struct! {
        #[derive(Clone, Debug, Default)]
        struct TestExtraData {
            multiplier: u64,
        }
    }

    impl IExtraData for TestExtraData {}

    struct TestProgram;

    impl SnarkAccountProgram for TestProgram {
        type State = TestState;
        type Msg = TestMsg;
        type ExtraData = TestExtraData;
        type Error = std::io::Error; // just anything

        fn process_message(
            &self,
            state: &mut Self::State,
            msg: InputMessage<Self::Msg>,
            _extra_data: &Self::ExtraData,
        ) -> ProgramResult<(), Self::Error> {
            if let InputMessage::Valid(_, m) = msg {
                state.value += m.delta;
            }
            Ok(())
        }

        fn finalize_state(
            &self,
            state: &mut Self::State,
            extra_data: Self::ExtraData,
        ) -> ProgramResult<(), Self::Error> {
            // Apply final multiplier
            state.value *= extra_data.multiplier;
            Ok(())
        }
    }

    impl SnarkAccountProgramVerification for TestProgram {
        type VState<'a> = u64; // Just track sum of deltas for verification
        type VInput<'a> = (); // No additional verification input needed for tests

        fn start_verification<'a>(
            &self,
            _state: &Self::State,
            _extra_data: &Self::ExtraData,
            _vinput: Self::VInput<'a>,
        ) -> ProgramResult<Self::VState<'a>, Self::Error> {
            Ok(0)
        }

        fn verify_coinput<'a>(
            &self,
            _state: &Self::State,
            vstate: &mut Self::VState<'a>,
            msg: &InputMessage<Self::Msg>,
            coinput: &[u8],
            _extra_data: &Self::ExtraData,
        ) -> ProgramResult<(), Self::Error> {
            // Require empty coinput for this test program
            if !coinput.is_empty() {
                return Err(ProgramError::MalformedCoinput);
            }

            // Track delta in vstate for verification
            if let InputMessage::Valid(_, m) = msg {
                *vstate += m.delta;
            }

            Ok(())
        }

        fn finalize_verification<'a>(
            &self,
            state: &Self::State,
            vstate: Self::VState<'a>,
            extra_data: &Self::ExtraData,
        ) -> ProgramResult<(), Self::Error> {
            // Verify that the accumulated deltas match expectation
            let expected = vstate * extra_data.multiplier;
            if state.value != expected && extra_data.multiplier != 0 {
                return Err(ProgramError::MismatchedState);
            }
            Ok(())
        }
    }

    fn make_valid_msg(delta: u64) -> InputMessage<TestMsg> {
        InputMessage::Valid(
            MsgMeta::new(AccountId::zero(), 0, BitcoinAmount::ZERO),
            TestMsg { delta },
        )
    }

    #[test]
    fn test_verify_and_apply_update_basic() {
        let program = TestProgram;
        let mut state = TestState { value: 0 };
        let messages = vec![make_valid_msg(5), make_valid_msg(3)];
        let coinputs: Vec<&[u8]> = vec![&[], &[]];
        let extra = TestExtraData { multiplier: 1 };

        let result = verify_and_apply_update(&program, &mut state, messages, coinputs, extra, ());
        assert!(result.is_ok());
        // (5 + 3) * 1 = 8
        assert_eq!(state.value, 8);
    }

    #[test]
    fn test_apply_unconditionally_basic() {
        let program = TestProgram;
        let mut state = TestState { value: 0 };
        let messages = vec![make_valid_msg(5), make_valid_msg(3)];
        let extra = TestExtraData { multiplier: 2 };

        let result = apply_update_unconditionally(&program, &mut state, messages, extra);
        assert!(result.is_ok());
        // (5 + 3) * 2 = 16
        assert_eq!(state.value, 16);
    }

    #[test]
    fn test_verify_fails_with_nonempty_coinput() {
        let program = TestProgram;
        let mut state = TestState { value: 0 };
        let messages = vec![make_valid_msg(5)];
        let coinputs: Vec<&[u8]> = vec![&[1, 2, 3]]; // Non-empty coinput
        let extra = TestExtraData { multiplier: 1 };

        let result = verify_and_apply_update(&program, &mut state, messages, coinputs, extra, ());
        assert!(matches!(
            result,
            Err(ProgramError::AtMessage { idx: 0, .. })
        ));
    }

    #[test]
    fn test_unknown_messages_processed() {
        use strata_acct_types::{AccountId, BitcoinAmount};

        use crate::MsgMeta;

        let program = TestProgram;
        let mut state = TestState { value: 0 };
        let messages = vec![
            make_valid_msg(5),
            InputMessage::Unknown(MsgMeta::new(AccountId::zero(), 0, BitcoinAmount::ZERO)),
            make_valid_msg(3),
        ];
        let coinputs: Vec<&[u8]> = vec![&[], &[], &[]];
        let extra = TestExtraData { multiplier: 1 };

        let result = verify_and_apply_update(&program, &mut state, messages, coinputs, extra, ());
        assert!(result.is_ok());
        // Only valid messages contribute: (5 + 3) * 1 = 8
        assert_eq!(state.value, 8);
    }
}
