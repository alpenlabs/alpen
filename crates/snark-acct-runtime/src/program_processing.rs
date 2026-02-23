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

use strata_codec::decode_buf_exact;

use crate::{
    IInnerState, InputMessage,
    errors::{ProgramError, ProgramResult},
    private_input::PrivateInput,
    traits::{SnarkAccountProgram, SnarkAccountProgramVerification},
};

pub fn process_update<'a, P: SnarkAccountProgramVerification>(
    program: &P,
    private_input: &PrivateInput,
    vinput: P::VInput<'a>,
) -> ProgramResult<(), P::Error> {
    // 1. Decode fields and verify consistency.
    let update = private_input.try_decode_update_pub_params()?;
    let mut state: P::State = private_input.try_decode_pre_state()?;
    if state.compute_state_root() != update.cur_state().inner_state() {
        return Err(ProgramError::MismatchedPreState);
    }

    let msg_count = update.message_inputs().len();
    if private_input.coinputs().len() != msg_count {
        return Err(ProgramError::MismatchedCoinputCount {
            expected: msg_count,
            actual: private_input.coinputs().len(),
        });
    }

    // TODO maybe we should remove the inbox indexes from the pub params?
    if update.cur_state().next_inbox_msg_idx() + msg_count as u64
        != update.new_state().next_inbox_msg_idx()
    {
        return Err(ProgramError::InconsistentMessageCount);
    }

    // 2. Decode extra data.
    let extra_data = decode_buf_exact::<P::ExtraData>(update.extra_data())
        .map_err(|_| ProgramError::MalformedExtraData)?;

    // 3. Create verification context and start verification.
    let mut vstate = program.start_verification(&state, &extra_data, vinput)?;
    program.start_update(&mut state, &extra_data)?;

    // 4. Process each message and coinput.
    for i in 0..msg_count {
        let msg_entry = &update.message_inputs()[i];
        let raw_coinp = private_input.coinputs()[i].raw_data();

        // Decode the message payload itself.
        let inp_msg = InputMessage::<P::Msg>::from_msg_entry(msg_entry);
        if !inp_msg.is_valid() && !raw_coinp.is_empty() {
            return Err(ProgramError::InvalidCoinput.at_msg(i));
        }

        // Verify the coinput against the message and then process the message itself.
        program
            .verify_coinput(&mut state, &mut vstate, &inp_msg, raw_coinp, &extra_data)
            .map_err(|e| e.at_msg(i))?;
        program
            .process_message(&mut state, inp_msg, &extra_data)
            .map_err(|e| e.at_msg(i))?;
    }

    // 5. Pre-finalize state to prepare for final verification.
    program.pre_finalize_state(&mut state, &extra_data)?;

    // 6. Final verification step.
    program.finalize_verification(&state, vstate, &extra_data)?;

    // 7. Final state update.
    program.finalize_state(&mut state, extra_data)?;

    // 8. Verify final state is consistent with proof.
    if state.compute_state_root() != update.new_state().inner_state() {
        return Err(ProgramError::MismatchedPostState);
    }

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

        let result =
            verify_and_apply_update_old(&program, &mut state, messages, coinputs, extra, ());
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

        let result =
            verify_and_apply_update_old(&program, &mut state, messages, coinputs, extra, ());
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

        let result =
            verify_and_apply_update_old(&program, &mut state, messages, coinputs, extra, ());
        assert!(result.is_ok());
        // Only valid messages contribute: (5 + 3) * 1 = 8
        assert_eq!(state.value, 8);
    }
}
