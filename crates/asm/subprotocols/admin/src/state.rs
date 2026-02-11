use std::mem::take;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_params::{AdministrationSubprotoParams, Role};
use strata_asm_txs_admin::actions::UpdateId;
use strata_crypto::threshold_signature::ThresholdConfigUpdate;

use crate::{
    authority::MultisigAuthority, error::AdministrationError, queued_update::QueuedUpdate,
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

    /// The confirmation depth (CD) setting, in Bitcoin blocks: after an update transaction
    /// receives this many confirmations, the update is enacted automatically. During this
    /// confirmation period, the update can still be cancelled by submitting a cancel transaction.
    confirmation_depth: u16,
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
            confirmation_depth: config.confirmation_depth,
        }
    }

    pub fn confirmation_depth(&self) -> u16 {
        self.confirmation_depth
    }

    /// Get a reference to the authority for the given role.
    pub fn authority(&self, role: Role) -> Option<&MultisigAuthority> {
        self.authorities.get(role as usize)
    }

    /// Get a mutable reference to the authority for the given role.
    pub fn authority_mut(&mut self, role: Role) -> Option<&mut MultisigAuthority> {
        self.authorities.get_mut(role as usize)
    }

    /// Apply a threshold config update for the specified role.
    pub fn apply_multisig_update(
        &mut self,
        role: Role,
        update: &ThresholdConfigUpdate,
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
        let (ready, rest): (Vec<_>, Vec<_>) = take(&mut self.queued)
            .into_iter()
            .partition(|u| u.activation_height() <= current_height);
        self.queued = rest;
        ready
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use rand::rngs::OsRng;
    use strata_asm_params::{AdministrationSubprotoParams, Role};
    use strata_asm_txs_admin::actions::UpdateAction;
    use strata_crypto::{
        keys::compressed::CompressedPublicKey,
        threshold_signature::{ThresholdConfig, ThresholdConfigUpdate},
    };
    use strata_test_utils::ArbitraryGenerator;

    use crate::{queued_update::QueuedUpdate, state::AdministrationSubprotoState};

    fn create_test_config() -> AdministrationSubprotoParams {
        let secp = Secp256k1::new();

        // Create admin keys
        let admin_sks: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
        let admin_pks: Vec<CompressedPublicKey> = admin_sks
            .iter()
            .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(&secp, sk)))
            .collect();
        let strata_administrator =
            ThresholdConfig::try_new(admin_pks, NonZero::new(2).unwrap()).unwrap();

        // Create seq manager keys
        let seq_sks: Vec<SecretKey> = (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();
        let seq_pks: Vec<CompressedPublicKey> = seq_sks
            .iter()
            .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(&secp, sk)))
            .collect();
        let strata_sequencer_manager =
            ThresholdConfig::try_new(seq_pks, NonZero::new(2).unwrap()).unwrap();

        AdministrationSubprotoParams {
            strata_administrator,
            strata_sequencer_manager,
            confirmation_depth: 2016,
        }
    }

    #[test]
    fn test_initial_state() {
        let config = create_test_config();
        let state = AdministrationSubprotoState::new(&config);

        assert_eq!(state.next_update_id(), 0);
        assert_eq!(state.queued().len(), 0);
    }

    #[test]
    fn test_enqueue_find_and_remove_queued() {
        let mut arb = ArbitraryGenerator::new();
        let config = create_test_config();
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
        let config = create_test_config();
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
        let secp = Secp256k1::new();
        let config = create_test_config();
        let mut state = AdministrationSubprotoState::new(&config);
        let role = Role::StrataAdministrator;

        let initial_auth = state.authority(role).unwrap().config();
        let initial_members: Vec<CompressedPublicKey> = initial_auth.keys().to_vec();

        // Generate new members to add
        let add_sks: Vec<SecretKey> = (0..2).map(|_| SecretKey::new(&mut OsRng)).collect();
        let add_members: Vec<CompressedPublicKey> = add_sks
            .iter()
            .map(|sk| CompressedPublicKey::from(PublicKey::from_secret_key(&secp, sk)))
            .collect();

        // Remove the first member
        let remove_members = vec![initial_members[0]];

        let new_size = initial_members.len() + add_members.len() - remove_members.len();
        let new_threshold = NonZero::new(2).unwrap();

        let update =
            ThresholdConfigUpdate::new(add_members.clone(), remove_members.clone(), new_threshold);

        state.apply_multisig_update(role, &update).unwrap();

        let updated_auth = state.authority(role).unwrap().config();

        // Verify threshold was updated
        assert_eq!(updated_auth.threshold(), new_threshold.get());

        // Verify size is correct
        assert_eq!(updated_auth.keys().len(), new_size);

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
