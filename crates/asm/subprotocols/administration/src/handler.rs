use strata_asm_common::MsgRelayer;
use strata_asm_proto_administration_txs::actions::{MultisigAction, UpdateAction};
use strata_crypto::multisig::vote::AggregatedVote;
use strata_primitives::roles::ProofType;

use crate::{
    error::AdministrationError, queued_update::QueuedUpdate, state::AdministrationSubprotoState,
};

pub(crate) fn handle_pending_updates(
    state: &mut AdministrationSubprotoState,
    _relayer: &mut impl MsgRelayer,
    current_height: u64,
) {
    // Get all the update actions that are ready to be enacted
    let actions_to_enact = state.process_queued(current_height);

    for action in actions_to_enact {
        match action.action() {
            UpdateAction::Multisig(update) => {
                state.apply_multisig_update(update.role(), update.config());
            }
            UpdateAction::VerifyingKey(update) => match update.kind() {
                ProofType::Asm => {
                    // TODO: STR-1721 Emit ASM Log
                }
                ProofType::OlStf => {
                    // TODO: STR-1721 Send a InterprotoMsg to Checkpoint subprotocol
                }
            },
            UpdateAction::OperatorSet(_update) => {
                // TODO: STR-1721 Set an InterProtoMsg to the Bridge Subprotocol;
            }
            UpdateAction::Sequencer(_update) => {
                // TODO: STF-1721 Send a InterprotoMsg to the Checkpoint subprotocol
            }
        }
    }
}

pub(crate) fn handle_action(
    state: &mut AdministrationSubprotoState,
    action: MultisigAction,
    vote: AggregatedVote,
    current_height: u64,
    _relayer: &mut impl MsgRelayer,
) -> Result<(), AdministrationError> {
    let role = match &action {
        MultisigAction::Update(update) => update.required_role(),
        MultisigAction::Cancel(cancel) => {
            let target_action_id = cancel.target_id();
            let queued = state
                .find_queued(target_action_id)
                .ok_or(AdministrationError::UnknownAction(*target_action_id))?;
            queued.action().required_role()
        }
    };

    let authority = state
        .authority(role)
        .ok_or(AdministrationError::UnknownRole)?;
    authority.validate_action(&action, &vote)?;

    match action {
        MultisigAction::Update(update) => {
            let id = state.next_update_id();
            match update {
                UpdateAction::Sequencer(_) => {
                    // TODO: directly apply it without queuing
                }
                action => {
                    // For all others add it to queue
                    let activation_height = current_height + state.confirmation_depth() as u64;
                    let queued_update = QueuedUpdate::new(id, action, activation_height);
                    state.enqueue(queued_update);
                }
            }
            state.increment_next_update_id();
        }
        MultisigAction::Cancel(cancel) => {
            state.remove_queued(cancel.target_id());
        }
    }

    // Increase the nonce
    let authority = state
        .authority_mut(role)
        .ok_or(AdministrationError::UnknownRole)?;
    authority.increment_seqno();

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    use rand::{seq::SliceRandom, thread_rng};
    use strata_asm_common::{AsmLogEntry, InterprotoMsg, MsgRelayer};
    use strata_asm_proto_administration_txs::actions::{
        CancelAction, MultisigAction, UpdateAction, updates::seq::SequencerUpdate,
    };
    use strata_crypto::multisig::{Signature, vote::AggregatedVote};
    use strata_primitives::roles::Role;
    use strata_test_utils::ArbitraryGenerator;

    use super::handle_action;
    use crate::{
        config::AdministrationSubprotoParams, error::AdministrationError,
        state::AdministrationSubprotoState,
    };

    struct MockRelayer {
        logs: Vec<AsmLogEntry>,
    }

    impl MockRelayer {
        fn new() -> Self {
            Self { logs: Vec::new() }
        }
    }

    impl MsgRelayer for MockRelayer {
        fn relay_msg(&mut self, _m: &dyn InterprotoMsg) {
            // Since we can't clone the dyn InterprotoMsg, just skip pushing messages in tests
            // self.messages.push(m.clone_box());
        }

        fn emit_log(&mut self, log: AsmLogEntry) {
            self.logs.push(log);
        }

        fn as_mut_any(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn create_test_state() -> AdministrationSubprotoState {
        let mut arb = ArbitraryGenerator::new();
        let config: AdministrationSubprotoParams = arb.generate();
        AdministrationSubprotoState::new(&config)
    }

    fn get_strata_administrator_update_actions(count: usize) -> Vec<UpdateAction> {
        let mut arb = ArbitraryGenerator::new();
        let mut actions = Vec::new();

        while actions.len() < count {
            let action: UpdateAction = arb.generate();
            if action.required_role() == Role::StrataAdministrator {
                actions.push(action);
            }
        }
        actions
    }

    /// Test that Strata Administrator update actions are properly handled:
    /// - Authority sequence number is incremented
    /// - Update ID is incremented
    /// - Actions are queued with correct activation height
    /// - Queued actions can be found in state
    #[test]
    fn test_strata_administrator_update_actions() {
        let mut state = create_test_state();
        let mut relayer = MockRelayer::new();
        let vote = AggregatedVote::new(vec![], Signature::default());
        let current_height = 1000;

        // Generate 5 random update actions that require StrataAdministrator role
        let updates = get_strata_administrator_update_actions(5);

        for update in updates {
            // Capture initial state before processing the update
            let initial_seq_no = state.authority(update.required_role()).unwrap().seqno();
            let initial_next_id = state.next_update_id();
            let initial_queued_len = state.queued().len();

            let action = MultisigAction::Update(update.clone());
            handle_action(
                &mut state,
                action,
                vote.clone(),
                current_height,
                &mut relayer,
            )
            .unwrap();

            // Verify state changes after processing
            let new_seq_no = state.authority(update.required_role()).unwrap().seqno();
            let new_next_id = state.next_update_id();
            let new_queued_len = state.queued().len();

            // Authority sequence number should increment by 1
            assert_eq!(new_seq_no, initial_seq_no + 1);
            // Next update ID should increment by 1
            assert_eq!(new_next_id, initial_next_id + 1);
            // Queue should contain one more item
            assert_eq!(new_queued_len, initial_queued_len + 1);

            // Verify the queued update has correct activation height
            let queued_update = state
                .find_queued(&initial_next_id)
                .expect("queued action must be found");

            assert_eq!(
                queued_update.activation_height(),
                current_height + state.confirmation_depth() as u64
            );
        }
    }

    /// Test that Sequencer update actions are handled differently from other updates:
    /// - Authority sequence number is incremented
    /// - Update ID is incremented
    /// - Actions are NOT queued (applied immediately)
    /// - No queued actions can be found in state
    #[test]
    fn test_strata_seq_manager_update_actions() {
        let mut arb = ArbitraryGenerator::new();

        let mut state = create_test_state();
        let mut relayer = MockRelayer::new();
        let vote = AggregatedVote::new(vec![], Signature::default());
        let current_height = 1000;

        // Generate random sequencer update actions
        let updates: Vec<SequencerUpdate> = arb.generate();

        for update in updates {
            let update: UpdateAction = update.into();
            // Capture initial state before processing the update
            let initial_seq_no = state.authority(update.required_role()).unwrap().seqno();
            let initial_next_id = state.next_update_id();
            let initial_queued_len = state.queued().len();
            let action = MultisigAction::Update(update.clone());
            handle_action(
                &mut state,
                action,
                vote.clone(),
                current_height,
                &mut relayer,
            )
            .unwrap();

            // Verify state changes after processing
            let new_seq_no = state.authority(update.required_role()).unwrap().seqno();
            let new_next_id = state.next_update_id();
            let new_queued_len = state.queued().len();

            // Authority sequence number should increment by 1
            assert_eq!(new_seq_no, initial_seq_no + 1);
            // Next update ID should increment by 1
            assert_eq!(new_next_id, initial_next_id + 1);
            // Queue length should remain the same (sequencer updates not queued)
            assert_eq!(new_queued_len, initial_queued_len);

            // Verify the update was not queued (applied immediately)
            assert!(state.find_queued(&initial_next_id).is_none());
        }
    }

    /// Test that cancel actions properly remove queued updates:
    /// - First queue 5 update actions.
    /// - Then cancel each one individually.
    /// - Verify sequence numbers increment, queue shrinks, and updates are removed.
    #[test]
    fn test_strata_administrator_cancel_action() {
        let mut state = create_test_state();
        let mut relayer = MockRelayer::new();
        let vote = AggregatedVote::new(vec![], Signature::default());
        let no_of_updates = 5;
        let current_height = 1000;

        // First, queue 5 update actions.
        let updates = get_strata_administrator_update_actions(no_of_updates);
        for update in updates {
            let update_action = MultisigAction::Update(update);
            handle_action(
                &mut state,
                update_action,
                vote.clone(),
                current_height,
                &mut relayer,
            )
            .unwrap();
        }

        // Then create a random order in which the actions are cancelled.
        let mut cancel_order: Vec<u32> = (0..no_of_updates as u32).collect();
        cancel_order.shuffle(&mut thread_rng());

        // Then cancel each queued update one by one based on the random order.
        for id in cancel_order {
            let cancel_action = MultisigAction::Cancel(CancelAction::new(id));
            let authorized_role = state.find_queued(&id).unwrap().action().required_role();
            // Capture initial state before cancellation
            let initial_seq_no = state.authority(authorized_role).unwrap().seqno();
            let initial_next_id = state.next_update_id();
            let initial_queued_len = state.queued().len();
            handle_action(
                &mut state,
                cancel_action,
                vote.clone(),
                current_height,
                &mut relayer,
            )
            .unwrap();
            // Verify state changes after cancellation
            let new_seq_no = state.authority(authorized_role).unwrap().seqno();
            let new_next_id = state.next_update_id();
            let new_queued_len = state.queued().len();

            // Authority sequence number should increment by 1
            assert_eq!(new_seq_no, initial_seq_no + 1);
            // Next update ID should remain unchanged (cancellation doesn't create new IDs)
            assert_eq!(new_next_id, initial_next_id);
            // Queue should shrink by 1
            assert_eq!(new_queued_len, initial_queued_len - 1);
            // The cancelled update should no longer be found
            assert!(state.find_queued(&id).is_none());
        }
    }

    /// Test that attempting to cancel a non-existent action returns an error:
    /// - Generate a random cancel action for an ID that doesn't exist
    /// - Verify that handle_action returns UnknownAction error
    #[test]
    fn test_strata_administrator_non_existent_cancel() {
        let mut arb = ArbitraryGenerator::new();
        let mut state = create_test_state();
        let mut relayer = MockRelayer::new();
        let vote = AggregatedVote::new(vec![], Signature::default());
        let current_height = 1000;

        // Generate a random cancel action (likely targeting a non-existent ID)
        let cancel_action: CancelAction = arb.generate();
        let cancel_action = MultisigAction::Cancel(cancel_action);

        // Attempt to cancel a non-existent action should return an error
        let res = handle_action(
            &mut state,
            cancel_action,
            vote.clone(),
            current_height,
            &mut relayer,
        );
        assert!(matches!(res, Err(AdministrationError::UnknownAction(_))));
    }

    /// Test that attempting to cancel a same action twice returns an error:
    /// - Generate a random update action and queue it.
    /// - Cancel the update action.
    /// - Verify that cancelling the update action again returns an UnknownAction error.
    #[test]
    fn test_strata_administrator_duplicate_cancels() {
        let mut state = create_test_state();
        let mut relayer = MockRelayer::new();
        let vote = AggregatedVote::new(vec![], Signature::default());
        let current_height = 1000;

        // Create an update action
        let update_id = state.next_update_id();
        let update = get_strata_administrator_update_actions(1)
            .first()
            .unwrap()
            .clone();
        let update_action = MultisigAction::Update(update);

        // Queue the update action
        handle_action(
            &mut state,
            update_action,
            vote.clone(),
            current_height,
            &mut relayer,
        )
        .unwrap();

        // Cancel the update action
        let cancel_action = MultisigAction::Cancel(CancelAction::new(update_id));
        let res = handle_action(
            &mut state,
            cancel_action,
            vote.clone(),
            current_height,
            &mut relayer,
        );
        assert!(res.is_ok());

        // Try cancelling the update action again
        let cancel_action = MultisigAction::Cancel(CancelAction::new(update_id));
        let res = handle_action(
            &mut state,
            cancel_action,
            vote.clone(),
            current_height,
            &mut relayer,
        );
        assert!(res.is_err());
        assert!(matches!(res, Err(AdministrationError::UnknownAction(_))));
    }
}
