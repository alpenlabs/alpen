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
use strata_snark_acct_types::{MessageEntry, UpdateManifest};

use crate::{
    ArchivedCoinput, ArchivedPrivateInput, IInnerState, InputMessage, UpdateLedgerInfo,
    errors::{ProgramError, ProgramResult},
    traits::{SnarkAccountProgram, SnarkAccountProgramVerification},
};

/// Verifies an update using proof's private inputs and a supplementary
/// verification input.
pub fn verify_and_process_update<'i, P: SnarkAccountProgramVerification>(
    program: &P,
    private_input: &ArchivedPrivateInput,
    vinput: P::VInput<'i>,
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

    // 3. Verify the update itself using the decoded structures.
    let ulinfo = UpdateLedgerInfo::from_update(&update);
    verify_update_inner(
        program,
        &mut state,
        vinput,
        update.message_inputs(),
        private_input.coinputs(),
        extra_data,
        ulinfo,
    )?;

    // 4. Verify final state is consistent with update.
    if state.compute_state_root() != update.new_state().inner_state() {
        return Err(ProgramError::MismatchedPostState);
    }

    Ok(())
}

fn verify_update_inner<'i, 'u, P: SnarkAccountProgramVerification>(
    program: &P,
    state: &mut P::State,
    vinput: P::VInput<'i>,
    messages: &[MessageEntry],
    coinputs: &[ArchivedCoinput],
    extra_data: P::ExtraData,
    ulinfo: UpdateLedgerInfo<'u>,
) -> ProgramResult<(), P::Error> {
    // 1. Create verification context and start verification.
    let mut vstate = program.start_verification(state, vinput, ulinfo)?;
    program.start_update(state)?;

    // 2. Process each message and coinput.
    for i in 0..messages.len() {
        verify_coinput_and_process_message(
            program,
            state,
            &mut vstate,
            &messages[i],
            coinputs[i].raw_data(),
        )
        .map_err(|e| e.at_msg(i))?;
    }

    // 3. Pre-finalize state to prepare for final verification.
    program.pre_finalize_state(state, &extra_data)?;

    // 4. Final verification step.
    program.finalize_verification(state, vstate, &extra_data)?;

    // 5. Final state changes.
    program.finalize_state(state, extra_data)?;

    Ok(())
}

fn verify_coinput_and_process_message<P: SnarkAccountProgramVerification>(
    program: &P,
    state: &mut P::State,
    vstate: &mut P::VState<'_>,
    msg_entry: &MessageEntry,
    raw_coinput: &[u8],
) -> ProgramResult<(), P::Error> {
    // 1. Decode the message payload, maybe erroring.
    let inp_msg = InputMessage::<P::Msg>::from_msg_entry(msg_entry);
    if !inp_msg.is_valid() && !raw_coinput.is_empty() {
        return Err(ProgramError::InvalidCoinput);
    }

    // 2. Verify the coinput against the message.
    program.verify_coinput(state, vstate, &inp_msg, raw_coinput)?;

    // 3. Process the message itself.
    program.process_message(state, inp_msg)?;

    Ok(())
}

/// Applies an update unconditionally without verification.
///
/// This is used outside the proof, after verifying the proof, to reconstruct
/// the actual state from DA.  It decodes the extra data and messages from the
/// [`UpdateManifest`] and applies them to the state, skipping coinput
/// verification and the `finalize_verification` step.
///
/// Correctness is implied by the orchestration layer permitting the state
/// transition in the first place, since that requires a snark proof.
pub fn apply_update_unconditionally<P: SnarkAccountProgram>(
    program: &P,
    state: &mut P::State,
    manifest: &UpdateManifest,
) -> ProgramResult<(), P::Error> {
    // 1. Decode extra data from the manifest.
    let extra_data = decode_buf_exact::<P::ExtraData>(manifest.extra_data())
        .map_err(|_| ProgramError::MalformedExtraData)?;

    // 2. Start update.
    program.start_update(state)?;

    // 3. Process messages without verification.
    for (idx, msg_entry) in manifest.messages().iter().enumerate() {
        let inp_msg = InputMessage::<P::Msg>::from_msg_entry(msg_entry);
        program
            .process_message(state, inp_msg)
            .map_err(|e| e.at_msg(idx))?;
    }

    // 4. Pre-finalize state.
    program.pre_finalize_state(state, &extra_data)?;

    // (5. Skip finalize_verification.)

    // 6. Finalize state.
    program.finalize_state(state, extra_data)?;

    // 7. Verify post-state matches manifest.
    if state.compute_state_root() != manifest.new_state().inner_state() {
        return Err(ProgramError::MismatchedPostState);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use rkyv::{rancor::Error as RkyvError, util::AlignedVec};
    use ssz_derive::{Decode, Encode};
    use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
    use strata_codec::impl_type_flat_struct;
    use strata_snark_acct_types::{
        LedgerRefs, MessageEntry, ProofState, UpdateManifest, UpdateOutputs,
    };

    use super::*;
    use crate::{
        UpdateLedgerInfo,
        private_input::Coinput,
        traits::{IAcctMsg, IExtraData, IInnerState},
    };

    /// Serializes a list of [`Coinput`]s with rkyv and returns the backing
    /// buffer.  Callers can access the archived slice via
    /// [`access_archived_coinputs`].
    fn archive_coinputs(coinputs: Vec<Coinput>) -> AlignedVec {
        rkyv::to_bytes::<RkyvError>(&coinputs).expect("rkyv encode coinputs")
    }

    /// Accesses the archived coinput slice from a buffer produced by
    /// [`archive_coinputs`].
    fn access_archived_coinputs(bytes: &AlignedVec) -> &[ArchivedCoinput] {
        // SAFETY: `bytes` was just produced by `rkyv::to_bytes` on a
        // `&[Coinput]` in the same test, so the root is valid.
        let archived = unsafe { rkyv::access_unchecked::<rkyv::Archived<Vec<Coinput>>>(bytes) };
        archived.as_slice()
    }

    // Simple test types for the generic processing functions.

    #[derive(Clone, Debug, Default, Encode, Decode)]
    struct TestState {
        value: u64,
    }

    impl IInnerState for TestState {
        fn compute_state_root(&self) -> Hash {
            let mut buf = [0u8; 32];
            buf[..8].copy_from_slice(&self.value.to_le_bytes());
            Hash::from(buf)
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

    #[expect(clippy::absolute_paths, reason = "conflicting imports")]
    impl SnarkAccountProgram for TestProgram {
        type State = TestState;
        type Msg = TestMsg;
        type ExtraData = TestExtraData;
        type Error = std::io::Error; // just anything

        fn process_message(
            &self,
            state: &mut Self::State,
            msg: InputMessage<Self::Msg>,
        ) -> ProgramResult<(), Self::Error> {
            if let Some(m) = msg.message() {
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

        fn start_verification<'i, 'u>(
            &self,
            _state: &Self::State,
            _vinput: Self::VInput<'i>,
            _ulinfo: UpdateLedgerInfo<'u>,
        ) -> ProgramResult<Self::VState<'i>, Self::Error> {
            Ok(0)
        }

        fn verify_coinput<'a>(
            &self,
            _state: &Self::State,
            vstate: &mut Self::VState<'a>,
            msg: &InputMessage<Self::Msg>,
            coinput: &[u8],
        ) -> ProgramResult<(), Self::Error> {
            // Require empty coinput for this test program
            if !coinput.is_empty() {
                return Err(ProgramError::MalformedCoinput);
            }

            // Track delta in vstate for verification
            if let Some(m) = msg.message() {
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
                return Err(ProgramError::MismatchedPostState);
            }
            Ok(())
        }
    }

    fn make_msg_entry(delta: u64) -> MessageEntry {
        let data = strata_codec::encode_to_vec(&TestMsg { delta }).unwrap();
        let payload = MsgPayload::new(BitcoinAmount::ZERO, data);
        MessageEntry::new(AccountId::zero(), 0, payload)
    }

    fn make_manifest(
        msg_entries: Vec<MessageEntry>,
        extra: TestExtraData,
        expected_post_value: u64,
    ) -> UpdateManifest {
        let extra_data = strata_codec::encode_to_vec(&extra).unwrap();
        let expected_state = TestState {
            value: expected_post_value,
        };
        let new_state = ProofState::new(expected_state.compute_state_root(), 0);
        UpdateManifest::new(new_state, extra_data, msg_entries)
    }

    #[test]
    fn test_verify_inner_basic() {
        let program = TestProgram;
        let mut state = TestState { value: 0 };
        let msg_entries = vec![make_msg_entry(5), make_msg_entry(3)];
        let coinputs_buf = archive_coinputs(vec![Coinput::new(vec![]), Coinput::new(vec![])]);
        let coinputs = access_archived_coinputs(&coinputs_buf);
        let extra = TestExtraData { multiplier: 1 };

        let lr = LedgerRefs::new_empty();
        let uo = UpdateOutputs::new_empty();
        let uli = UpdateLedgerInfo::new(&lr, &uo);

        let result =
            verify_update_inner(&program, &mut state, (), &msg_entries, coinputs, extra, uli);
        assert!(result.is_ok());
        // (5 + 3) * 1 = 8
        assert_eq!(state.value, 8);
    }

    #[test]
    fn test_apply_unconditionally_basic() {
        let program = TestProgram;
        let mut state = TestState { value: 0 };
        // (5 + 3) * 2 = 16
        let manifest = make_manifest(
            vec![make_msg_entry(5), make_msg_entry(3)],
            TestExtraData { multiplier: 2 },
            16,
        );

        let result = apply_update_unconditionally(&program, &mut state, &manifest);
        assert!(result.is_ok());
        assert_eq!(state.value, 16);
    }

    #[test]
    fn test_verify_fails_with_nonempty_coinput() {
        let program = TestProgram;
        let mut state = TestState { value: 0 };
        let msg_entries = vec![make_msg_entry(5)];
        let coinputs_buf = archive_coinputs(vec![Coinput::new(vec![1, 2, 3])]); // Non-empty
        let coinputs = access_archived_coinputs(&coinputs_buf);
        let extra = TestExtraData { multiplier: 1 };

        let lr = LedgerRefs::new_empty();
        let uo = UpdateOutputs::new_empty();
        let uli = UpdateLedgerInfo::new(&lr, &uo);

        let result =
            verify_update_inner(&program, &mut state, (), &msg_entries, coinputs, extra, uli);
        assert!(matches!(
            result,
            Err(ProgramError::AtMessage { idx: 0, .. })
        ));
    }

    #[test]
    fn test_unknown_messages_processed() {
        let program = TestProgram;
        let mut state = TestState { value: 0 };

        // Create a message entry with garbage payload data to trigger Unknown
        let unknown_payload = MsgPayload::new(BitcoinAmount::ZERO, vec![0xff]);
        let unknown_entry = MessageEntry::new(AccountId::zero(), 0, unknown_payload);

        // Only valid messages contribute: (5 + 3) * 1 = 8
        let manifest = make_manifest(
            vec![make_msg_entry(5), unknown_entry, make_msg_entry(3)],
            TestExtraData { multiplier: 1 },
            8,
        );

        let result = apply_update_unconditionally(&program, &mut state, &manifest);
        assert!(result.is_ok());
        assert_eq!(state.value, 8);
    }
}
