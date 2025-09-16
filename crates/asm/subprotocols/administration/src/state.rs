use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_administration_txs::actions::UpdateId;
use strata_crypto::multisig::SchnorrMultisigConfigUpdate;
use strata_primitives::roles::Role;

use crate::{
    authority::MultisigAuthority, config::AdministrationSubprotoParams, error::AdministrationError,
    queued_update::QueuedUpdate,
};

/// Holds the state for the Administration Subprotocol, including the various
/// multisignature authorities and any actions still pending execution.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct AdministrationSubprotoState {
    /// List of configurations for multisignature authorities.
    /// Each entry specifies who the signers are and how many signatures
    /// are required to approve an action.
    authorities: Vec<MultisigAuthority>,

    /// List of updates that have been queued for execution.
    /// These remain in a queued state and can be cancelled via a CancelTx until execution. If not
    /// cancelled, they are executed automatically once their activation height is reached.
    queued: Vec<QueuedUpdate>,

    /// UpdateId for the next update.
    next_update_id: UpdateId,
}

impl AdministrationSubprotoState {
    pub fn new(config: &AdministrationSubprotoParams) -> Self {
        let authorities = config
            .clone()
            .get_all_authorities()
            .into_iter()
            .map(|(role, config)| MultisigAuthority::new(role, config))
            .collect();

        Self {
            authorities,
            queued: Vec::new(),
            next_update_id: 0,
        }
    }
    /// Get a reference to the authority for the given role.
    pub fn authority(&self, role: Role) -> Option<&MultisigAuthority> {
        self.authorities.get(role as usize)
    }

    /// Get a mutable reference to the authority for the given role.
    pub fn authority_mut(&mut self, role: Role) -> Option<&mut MultisigAuthority> {
        self.authorities.get_mut(role as usize)
    }

    /// Apply a multisig config update for the specified role.
    pub fn apply_multisig_update(
        &mut self,
        role: Role,
        update: &SchnorrMultisigConfigUpdate,
    ) -> Result<(), AdministrationError> {
        if let Some(auth) = self.authority_mut(role) {
            auth.config_mut().apply_update(update)?;
            Ok(())
        } else {
            Err(AdministrationError::UnknownRole)
        }
    }

    /// Get a reference to the queued updates.
    pub fn queued(&self) -> &[QueuedUpdate] {
        &self.queued
    }

    /// Find a queued update by its ID.
    pub fn find_queued(&self, id: &UpdateId) -> Option<&QueuedUpdate> {
        self.queued.iter().find(|u| u.id() == id)
    }

    /// Queue a new update.
    pub fn enqueue(&mut self, update: QueuedUpdate) {
        self.queued.push(update);
    }

    /// Remove a queued update by swapping it out.
    pub fn remove_queued(&mut self, id: &UpdateId) {
        if let Some(i) = self.queued.iter().position(|u| u.id() == id) {
            self.queued.swap_remove(i);
        }
    }

    /// Get the next global update id.
    pub fn next_update_id(&self) -> UpdateId {
        self.next_update_id
    }

    /// Increment the next global update id.
    pub fn increment_next_update_id(&mut self) {
        self.next_update_id += 1;
    }

    /// Process all queued updates and remove any whose `activation_height` equals `current_height`
    /// from `queued`.
    pub fn process_queued(&mut self, current_height: u64) -> Vec<QueuedUpdate> {
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queued)
            .into_iter()
            .partition(|u| u.activation_height() <= current_height);
        self.queued = rest;
        ready
    }
}

#[cfg(test)]
mod tests {
    use rand::{Rng, thread_rng};
    use strata_asm_proto_administration_txs::actions::UpdateAction;
    use strata_crypto::multisig::config::MultisigConfigUpdate;
    use strata_primitives::{buf::Buf32, roles::Role};
    use strata_test_utils::ArbitraryGenerator;

    use crate::{
        config::AdministrationSubprotoParams, queued_update::QueuedUpdate,
        state::AdministrationSubprotoState,
    };

    #[test]
    fn test_initial_state() {
        let mut arb = ArbitraryGenerator::new();
        let config: AdministrationSubprotoParams = arb.generate();
        let state = AdministrationSubprotoState::new(&config);

        assert_eq!(state.next_update_id(), 0);
        assert_eq!(state.queued().len(), 0);
    }

    #[test]
    fn test_enqueue_find_and_remove_queued() {
        let mut arb = ArbitraryGenerator::new();
        let config: AdministrationSubprotoParams = arb.generate();
        let mut state = AdministrationSubprotoState::new(&config);

        // Use arbitrary action or fallback to guaranteed queueable action
        let update: QueuedUpdate = arb.generate();
        let update_id = *update.id();

        state.enqueue(update.clone());

        assert_eq!(state.find_queued(&update_id), Some(&update));
        assert_eq!(state.find_queued(&(update_id + 1)), None);

        state.remove_queued(&update_id);
        assert_eq!(state.find_queued(&update_id), None);
    }

    /// Helper to seed queued updates with specific activation heights
    fn seed_queued(ids: &[u32], activation_heights: &[u64]) -> AdministrationSubprotoState {
        let mut arb = ArbitraryGenerator::new();
        let config = arb.generate();
        let mut state = AdministrationSubprotoState::new(&config);

        for (&id, &activation_height) in ids.iter().zip(activation_heights.iter()) {
            let update: UpdateAction = arb.generate();
            let queued_update = QueuedUpdate::new(id, update, activation_height);
            state.enqueue(queued_update);
        }
        state
    }

    #[test]
    fn test_process_queued_table() {
        struct Case {
            current: u64,
            want_queued: Vec<u32>,
            want_ready: Vec<u32>,
        }

        let ids = &[1, 2, 3];
        let activation_heights = &[5000, 5100, 5200];

        let cases = vec![
            Case {
                current: 4999,
                want_queued: vec![1, 2, 3],
                want_ready: vec![],
            },
            Case {
                current: 5000,
                want_queued: vec![2, 3],
                want_ready: vec![1],
            },
            Case {
                current: 5100,
                want_queued: vec![3],
                want_ready: vec![1, 2],
            },
            Case {
                current: 5200,
                want_queued: vec![],
                want_ready: vec![1, 2, 3],
            },
        ];

        for case in cases {
            let mut state = seed_queued(ids, activation_heights);
            let ready_updates = state.process_queued(case.current);

            let mut queued_ids: Vec<_> = state.queued.iter().map(|u| *u.id()).collect();
            queued_ids.sort_unstable();

            let mut ready_ids: Vec<_> = ready_updates.iter().map(|u| *u.id()).collect();
            ready_ids.sort_unstable();

            assert_eq!(
                queued_ids, case.want_queued,
                "at height {} queued mismatch",
                case.current
            );
            assert_eq!(
                ready_ids, case.want_ready,
                "at height {} ready mismatch",
                case.current
            );
        }
    }

    #[test]
    fn test_apply_multisig_update() {
        let mut arb = ArbitraryGenerator::new();
        let config: AdministrationSubprotoParams = arb.generate();
        let mut state = AdministrationSubprotoState::new(&config);
        let role: Role = arb.generate();

        let initial_auth = state.authority(role).unwrap().config();
        let initial_members: Vec<Buf32> = initial_auth.keys().to_vec();

        let add_members: Vec<Buf32> = arb.generate();

        // Randomly pick some members to remove
        let mut rng = thread_rng();
        let mut remove_members = Vec::new();

        for member in &initial_members {
            if rng.gen_bool(0.3) {
                // 30% chance to remove each member
                remove_members.push(*member);
            }
        }

        let new_size = initial_members.len() + add_members.len() - remove_members.len();
        let new_threshold = rng.gen_range(1..=new_size);

        let update = MultisigConfigUpdate::new(
            add_members.clone(),
            remove_members.clone(),
            new_threshold as u8,
        );

        state.apply_multisig_update(role, &update).unwrap();

        let updated_auth = state.authority(role).unwrap().config();

        // Verify threshold was updated
        assert_eq!(updated_auth.threshold(), new_threshold as u8);

        // Verify that specified members were removed
        for member_to_remove in &remove_members {
            assert!(
                !updated_auth.keys().contains(member_to_remove),
                "Member {:?} was not removed",
                member_to_remove
            );
        }

        // Verify that new members were added
        for new_member in &add_members {
            assert!(
                updated_auth.keys().contains(new_member),
                "New member {:?} was not added",
                new_member
            );
        }
    }
}
