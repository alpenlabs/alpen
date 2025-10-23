use strata_asm_common::{
    MsgRelayer,
    logging::{error, info},
};
use strata_asm_proto_checkpoint_v0::CheckpointIncomingMsg;
use strata_asm_txs_admin::actions::{MultisigAction, UpdateAction};
use strata_crypto::multisig::SchnorrMultisigSignature;
use strata_predicate::PredicateKey;
use strata_primitives::{buf::Buf32, roles::ProofType};

use crate::{
    config::AdministrationSubprotoParams, error::AdministrationError, queued_update::QueuedUpdate,
    state::AdministrationSubprotoState,
};

/// Processes and applies all queued updates that are ready to be enacted at the current height.
///
/// This function retrieves all update actions from the queue that are ready to be applied
/// and processes them sequentially. If an error occurs during the execution of any update,
/// an error log is emitted and processing continues with the next queued update.
///
/// This function should not return an error - it handles all errors internally by logging
/// them and continuing with the next update to ensure system resilience.
pub(crate) fn handle_pending_updates(
    state: &mut AdministrationSubprotoState,
    relayer: &mut impl MsgRelayer,
    current_height: u64,
) {
    // Get all the update actions that are ready to be enacted
    let queued_updates = state.process_queued(current_height);
    for queued in queued_updates {
        let (update_id, action) = queued.into_id_and_action();

        match action {
            UpdateAction::Multisig(update) => {
                match state.apply_multisig_update(update.role(), update.config()) {
                    Ok(_) => {
                        info!(
                            update_id = update_id,
                            "Successfully applied multisig update to role {:?}",
                            update.role(),
                        );
                    }
                    Err(e) => {
                        error!(
                            update_id = update_id,
                            "Failed to apply multisig update to role {:?}: {}",
                            update.role(),
                            e,
                        );
                    }
                }
            }
            UpdateAction::VerifyingKey(update) => {
                let (key, kind) = update.into_inner();
                match kind {
                    ProofType::Asm => {
                        // TODO: STR-1721 Emit ASM Log
                    }
                    ProofType::OlStf => {
                        relay_checkpoint_predicate(relayer, key);
                        info!(
                            %update_id,
                            "Forwarded rollup verifying key update to checkpoint subprotocol",
                        );
                    }
                }
            }
            UpdateAction::OperatorSet(_update) => {
                // TODO: STR-1721 Set an InterProtoMsg to the Bridge Subprotocol
            }
            UpdateAction::Sequencer(update) => {
                let new_key = update.into_inner();
                relay_checkpoint_sequencer_update(relayer, new_key);
                info!(
                    update_id = update_id,
                    "Forwarded queued sequencer key update to checkpoint subprotocol",
                );
            }
        }
    }
}

/// Processes a multisig action (an admin "change" message) by validating the aggregated signature
/// and executing the requested operation.
///
/// This function handles the complete lifecycle of a multisig action:
/// 1. Determines the required role based on the action type
/// 2. Validates that the aggregated signature meets the multisig requirements for that role
/// 3. Processes the action based on its type:
///    - `Update`: Queues the action for later execution (except sequencer updates which apply
///      immediately)
///    - `Cancel`: Removes a previously queued action from the queue
/// 4. Increments the authority's sequence number to prevent replay attacks
///
/// # Returns
/// * `Ok(())` if the action was successfully processed
/// * `Err(AdministrationError)` if validation failed or the action could not be processed
pub(crate) fn handle_action(
    state: &mut AdministrationSubprotoState,
    action: MultisigAction,
    sig: SchnorrMultisigSignature,
    current_height: u64,
    relayer: &mut impl MsgRelayer,
    params: &AdministrationSubprotoParams,
) -> Result<(), AdministrationError> {
    // Determine the required role based on the action type
    let role = match &action {
        MultisigAction::Update(update) => update.required_role(),
        MultisigAction::Cancel(cancel) => {
            // For cancel actions, we need to find the target action to determine its required role
            let target_action_id = cancel.target_id();
            let queued = state
                .find_queued(target_action_id)
                .ok_or(AdministrationError::UnknownAction(*target_action_id))?;
            queued.action().required_role()
        }
    };

    // Get the authority for this role and validate the action with the aggregated signature
    let authority = state
        .authority(role)
        .ok_or(AdministrationError::UnknownRole)?;
    authority.verify_action_signature(&action, &sig)?;

    // Process the action based on its type
    match action {
        MultisigAction::Update(update) => {
            // Generate a unique ID for this update
            let id = state.next_update_id();
            match update {
                // Directly apply it without queuing
                UpdateAction::Sequencer(update) => {
                    let new_key = update.into_inner();
                    relay_checkpoint_sequencer_update(relayer, new_key);
                    info!(
                        update_id = id,
                        "Forwarded sequencer key update immediately to checkpoint subprotocol",
                    );
                }
                action => {
                    // For all other update types, add to the queue with a future activation height
                    let activation_height = current_height + params.confirmation_depth as u64;
                    let queued_update = QueuedUpdate::new(id, action, activation_height);
                    state.enqueue(queued_update);
                }
            }
            // Increment the update ID counter for the next action
            state.increment_next_update_id();
        }
        MultisigAction::Cancel(cancel) => {
            // Remove the target action from the queue
            state.remove_queued(cancel.target_id());
        }
    }

    // Increment the sequence number for the authority to prevent replay attacks
    let authority = state
        .authority_mut(role)
        .ok_or(AdministrationError::UnknownRole)?;
    authority.increment_seqno();

    Ok(())
}

fn relay_checkpoint_sequencer_update(relayer: &mut impl MsgRelayer, new_key: Buf32) {
    let msg = CheckpointIncomingMsg::UpdateSequencerKey(new_key);
    relayer.relay_msg(&msg);
}

fn relay_checkpoint_predicate(relayer: &mut impl MsgRelayer, key: PredicateKey) {
    let msg = CheckpointIncomingMsg::UpdateCheckpointPredicate(key);
    relayer.relay_msg(&msg);
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    use bitcoin::secp256k1::{SECP256K1, SecretKey};
    use bitvec::prelude::*;
    use rand::{rngs::OsRng, seq::SliceRandom, thread_rng};
    use strata_asm_common::{AsmLogEntry, InterprotoMsg, MsgRelayer};
    use strata_asm_proto_checkpoint_v0::CheckpointIncomingMsg;
    use strata_asm_txs_admin::{
        actions::{
            CancelAction, MultisigAction, UpdateAction,
            updates::{predicate::PredicateUpdate, seq::SequencerUpdate},
        },
        test_utils::create_multisig_signature,
    };
    use strata_crypto::{
        EvenSecretKey,
        multisig::{
            MultisigError, SchnorrMultisigConfig, SchnorrScheme, signature::AggregatedSignature,
        },
    };
    use strata_predicate::PredicateKey;
    use strata_primitives::{
        buf::{Buf32, Buf64},
        roles::{ProofType, Role},
    };
    use strata_test_utils::ArbitraryGenerator;

    use super::{handle_action, handle_pending_updates};
    use crate::{
        config::AdministrationSubprotoParams, error::AdministrationError,
        queued_update::QueuedUpdate, state::AdministrationSubprotoState,
    };

    struct MockRelayer<M> {
        logs: Vec<AsmLogEntry>,
        messages: Vec<M>,
    }

    impl<M> MockRelayer<M> {
        fn new() -> Self {
            Self {
                logs: Vec::new(),
                messages: Vec::new(),
            }
        }

        fn messages(&self) -> &[M] {
            &self.messages
        }
    }

    impl<M> MsgRelayer for MockRelayer<M>
    where
        M: InterprotoMsg + Clone + 'static,
    {
        fn relay_msg(&mut self, m: &dyn InterprotoMsg) {
            if let Some(msg) = m.as_dyn_any().downcast_ref::<M>() {
                self.messages.push(msg.clone());
            }
        }

        fn emit_log(&mut self, log: AsmLogEntry) {
            self.logs.push(log);
        }

        fn as_mut_any(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn create_test_params() -> (
        AdministrationSubprotoParams,
        Vec<EvenSecretKey>,
        Vec<EvenSecretKey>,
    ) {
        let strata_admin_sks: Vec<EvenSecretKey> =
            (0..3).map(|_| SecretKey::new(&mut OsRng).into()).collect();
        let strata_admin_pks: Vec<Buf32> = strata_admin_sks
            .iter()
            .map(|sk| sk.x_only_public_key(SECP256K1).0.into())
            .collect();
        let strata_administrator = SchnorrMultisigConfig::try_new(strata_admin_pks, 2).unwrap();

        let strata_seq_manager_sks: Vec<EvenSecretKey> =
            (0..3).map(|_| SecretKey::new(&mut OsRng).into()).collect();
        let strata_seq_manager_pks: Vec<Buf32> = strata_seq_manager_sks
            .iter()
            .map(|sk| sk.x_only_public_key(SECP256K1).0.into())
            .collect();
        let strata_sequencer_manager =
            SchnorrMultisigConfig::try_new(strata_seq_manager_pks, 2).unwrap();

        let config = AdministrationSubprotoParams {
            strata_administrator,
            strata_sequencer_manager,
            confirmation_depth: 2016,
        };

        (config, strata_admin_sks, strata_seq_manager_sks)
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
        let (params, admin_sks, _) = create_test_params();
        let mut state = AdministrationSubprotoState::new(&params);
        let mut relayer = MockRelayer::<CheckpointIncomingMsg>::new();
        let current_height = 1000;

        // Generate 5 random update actions that require StrataAdministrator role
        let updates = get_strata_administrator_update_actions(5);

        // Create signer indices (signers 0 and 2)
        let signer_indices = bitvec![u8, Lsb0; 1, 0, 1];

        for update in updates {
            // Capture initial state before processing the update
            let initial_seq_no = state.authority(update.required_role()).unwrap().seqno();
            let initial_next_id = state.next_update_id();
            let initial_queued_len = state.queued().len();

            let action = MultisigAction::Update(update.clone());
            let sighash = action.compute_sighash(initial_seq_no);
            let multisig = create_multisig_signature(&admin_sks, signer_indices.clone(), sighash);
            handle_action(
                &mut state,
                action,
                multisig,
                current_height,
                &mut relayer,
                &params,
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
                current_height + params.confirmation_depth as u64
            );
        }
    }

    /// Test that multisig actions reject invalid sequence numbers.
    ///
    /// Verifies that sequence number validation prevents replay attacks by rejecting
    /// duplicate and out-of-order sequence numbers for StrataAdministrator actions.
    #[test]
    fn test_strata_administrator_incorrect_seqno() {
        let (params, admin_sks, _) = create_test_params();
        let mut state = AdministrationSubprotoState::new(&params);
        let mut relayer = MockRelayer::<CheckpointIncomingMsg>::new();
        let current_height = 1000;
        let initial_seq_no = 0;

        // Generate a random update action that require StrataAdministrator role
        let update = get_strata_administrator_update_actions(1)[0].clone();

        // Create signer indices (signers 0 and 2)
        let signer_indices = bitvec![u8, Lsb0; 1, 0, 1];

        // Create an action and queue that.
        let action = MultisigAction::Update(update.clone());
        let sighash = action.compute_sighash(initial_seq_no);
        let multisig = create_multisig_signature(&admin_sks, signer_indices.clone(), sighash);
        let res = handle_action(
            &mut state,
            action,
            multisig,
            current_height,
            &mut relayer,
            &params,
        );
        assert!(res.is_ok());

        // Try queuing it again with same seq no
        let action = MultisigAction::Update(update.clone());
        let sighash = action.compute_sighash(initial_seq_no);
        let multisig = create_multisig_signature(&admin_sks, signer_indices.clone(), sighash);
        let res = handle_action(
            &mut state,
            action,
            multisig,
            current_height,
            &mut relayer,
            &params,
        );
        assert!(res.is_err());
        assert!(matches!(
            res,
            Err(AdministrationError::Multisig(
                MultisigError::InvalidSignature
            ))
        ));

        // Try queuing it again with same arbitrary seq no.
        let seq_no: u64 = ArbitraryGenerator::new().generate();
        let action = MultisigAction::Update(update.clone());
        let sighash = action.compute_sighash(seq_no);
        let multisig = create_multisig_signature(&admin_sks, signer_indices.clone(), sighash);
        let res = handle_action(
            &mut state,
            action,
            multisig,
            current_height,
            &mut relayer,
            &params,
        );
        assert!(matches!(
            res,
            Err(AdministrationError::Multisig(
                MultisigError::InvalidSignature
            ))
        ));
    }

    /// Test that Sequencer update actions are handled differently from other updates:
    /// - Authority sequence number is incremented
    /// - Update ID is incremented
    /// - Actions are NOT queued (applied immediately)
    /// - No queued actions can be found in state
    #[test]
    fn test_strata_seq_manager_update_actions() {
        let mut arb = ArbitraryGenerator::new();
        let (params, _, seq_manager_sks) = create_test_params();
        let mut state = AdministrationSubprotoState::new(&params);

        let mut relayer = MockRelayer::<CheckpointIncomingMsg>::new();
        let current_height = 1000;

        // Generate random sequencer update actions
        let updates: Vec<SequencerUpdate> = arb.generate();
        let update_count = updates.len();

        // Create signer indices (signers 0 and 2)
        let signer_indices = bitvec![u8, Lsb0; 1, 0, 1];

        for update in updates {
            let update: UpdateAction = update.into();
            // Capture initial state before processing the update
            let initial_seq_no = state.authority(update.required_role()).unwrap().seqno();
            let initial_next_id = state.next_update_id();
            let initial_queued_len = state.queued().len();

            let action = MultisigAction::Update(update.clone());
            let sighash = action.compute_sighash(initial_seq_no);
            let multisig =
                create_multisig_signature(&seq_manager_sks, signer_indices.clone(), sighash);

            handle_action(
                &mut state,
                action,
                multisig,
                current_height,
                &mut relayer,
                &params,
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

        let checkpoint_msgs = relayer.messages();
        assert_eq!(checkpoint_msgs.len(), update_count);
        assert!(
            checkpoint_msgs
                .iter()
                .all(|msg| matches!(msg, CheckpointIncomingMsg::UpdateSequencerKey(_)))
        );
    }

    #[test]
    fn test_rollup_verifying_key_update_forwarded_to_checkpoint() {
        let (params, _, _) = create_test_params();
        let mut state = AdministrationSubprotoState::new(&params);
        let mut relayer = MockRelayer::<CheckpointIncomingMsg>::new();

        let predicate = PredicateKey::always_accept();

        let update = PredicateUpdate::new(predicate.clone(), ProofType::OlStf);
        let update_id = state.next_update_id();
        let activation_height = 42;
        state.enqueue(QueuedUpdate::new(
            update_id,
            update.into(),
            activation_height,
        ));

        handle_pending_updates(&mut state, &mut relayer, activation_height);

        assert!(state.queued().is_empty());
        let checkpoint_msgs = relayer.messages();
        assert_eq!(checkpoint_msgs.len(), 1);
        match checkpoint_msgs
            .first()
            .expect("checkpoint message expected")
        {
            CheckpointIncomingMsg::UpdateCheckpointPredicate(incoming_predicate) => {
                assert_eq!(incoming_predicate, &predicate);
            }
            _ => panic!("expected rollup verifying key update to checkpoint"),
        }
    }

    /// Test that cancel actions properly remove queued updates:
    /// - First queue 5 update actions.
    /// - Then cancel each one individually.
    /// - Verify sequence numbers increment, queue shrinks, and updates are removed.
    #[test]
    fn test_strata_administrator_cancel_action() {
        let (params, admin_sks, _) = create_test_params();
        let mut state = AdministrationSubprotoState::new(&params);
        let mut relayer = MockRelayer::<CheckpointIncomingMsg>::new();
        let no_of_updates = 5;
        let current_height = 1000;

        // create signer indices (signers 0 and 2)
        let signer_indices = bitvec![u8, Lsb0; 1, 0, 1];

        // First, queue 5 update actions
        let updates = get_strata_administrator_update_actions(no_of_updates);

        for update in updates {
            let seq_no = state.authority(update.required_role()).unwrap().seqno();
            let update_action = MultisigAction::Update(update);

            let sighash = update_action.compute_sighash(seq_no);
            let multisig = create_multisig_signature(&admin_sks, signer_indices.clone(), sighash);

            handle_action(
                &mut state,
                update_action,
                multisig,
                current_height,
                &mut relayer,
                &params,
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

            let sighash = cancel_action.compute_sighash(initial_seq_no);
            let multisig = create_multisig_signature(&admin_sks, signer_indices.clone(), sighash);

            handle_action(
                &mut state,
                cancel_action,
                multisig,
                current_height,
                &mut relayer,
                &params,
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
        let (params, _, _) = create_test_params();
        let mut state = AdministrationSubprotoState::new(&params);
        let mut relayer = MockRelayer::<CheckpointIncomingMsg>::new();
        let multisig = AggregatedSignature::<SchnorrScheme>::new(BitVec::new(), Buf64::default());
        let current_height = 1000;

        // Generate a random cancel action (likely targeting a non-existent ID)
        let cancel_action: CancelAction = arb.generate();
        let cancel_action = MultisigAction::Cancel(cancel_action);

        // Attempt to cancel a non-existent action should return an error
        let res = handle_action(
            &mut state,
            cancel_action,
            multisig,
            current_height,
            &mut relayer,
            &params,
        );
        assert!(matches!(res, Err(AdministrationError::UnknownAction(_))));
    }

    /// Test that attempting to cancel a same action twice returns an error:
    /// - Generate a random update action and queue it.
    /// - Cancel the update action.
    /// - Verify that cancelling the update action again returns an UnknownAction error.
    #[test]
    fn test_strata_administrator_duplicate_cancels() {
        let (params, admin_sks, _) = create_test_params();
        let mut relayer = MockRelayer::<CheckpointIncomingMsg>::new();
        let mut state = AdministrationSubprotoState::new(&params);
        let initial_seq_no = 0;
        let current_height = 1000;

        // Create an update action
        let update_id = state.next_update_id();
        let update = get_strata_administrator_update_actions(1)
            .first()
            .unwrap()
            .clone();
        let update_action = MultisigAction::Update(update);

        // create signer indices (signers 0 and 2)
        let signer_indices = bitvec![u8, Lsb0; 1, 0, 1];

        let sighash = update_action.compute_sighash(initial_seq_no);
        let multisig = create_multisig_signature(&admin_sks, signer_indices.clone(), sighash);

        // Queue the update action
        handle_action(
            &mut state,
            update_action,
            multisig.clone(),
            current_height,
            &mut relayer,
            &params,
        )
        .unwrap();

        // Cancel the update action
        let cancel_action = MultisigAction::Cancel(CancelAction::new(update_id));
        let sighash = cancel_action.compute_sighash(initial_seq_no + 1);
        let multisig = create_multisig_signature(&admin_sks, signer_indices.clone(), sighash);
        let res = handle_action(
            &mut state,
            cancel_action,
            multisig.clone(),
            current_height,
            &mut relayer,
            &params,
        );
        assert!(res.is_ok());

        // Try cancelling the update action again
        let cancel_action = MultisigAction::Cancel(CancelAction::new(update_id));
        let res = handle_action(
            &mut state,
            cancel_action,
            multisig.clone(),
            current_height,
            &mut relayer,
            &params,
        );
        assert!(res.is_err());
        assert!(matches!(res, Err(AdministrationError::UnknownAction(_))));
    }
}
