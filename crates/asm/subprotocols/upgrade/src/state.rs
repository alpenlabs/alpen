use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::actions::UpdateId;
use strata_crypto::multisig::config::MultisigConfigUpdate;
use strata_primitives::roles::Role;

use crate::{
    authority::MultisigAuthority,
    config::UpgradeSubprotoConfig,
    updates::{committed::CommittedUpdate, queued::QueuedUpdate, scheduled::ScheduledUpdate},
};

/// Holds the state for the Upgrade Subprotocol, including the various
/// multisignature authorities and any actions still pending execution.
#[derive(Debug, Clone, Eq, PartialEq, Default, BorshSerialize, BorshDeserialize)]
pub struct UpgradeSubprotoState {
    /// List of configurations for multisignature authorities.
    /// Each entry specifies who the signers are and how many signatures
    /// are required to approve an action.
    authorities: Vec<MultisigAuthority>,

    /// Actions that can still be cancelled by CancelTx while waiting
    /// for their block countdown to complete.
    queued: Vec<QueuedUpdate>,

    /// Actions that have completed their waiting period and can be
    /// enacted by EnactmentTx, but can no longer be cancelled.
    committed: Vec<CommittedUpdate>,

    /// Actions that will be executed automatically without requiring
    /// an EnactmentTx transaction.
    scheduled: Vec<ScheduledUpdate>,

    /// UpdateId for the next update
    next_update_id: UpdateId,
}

impl UpgradeSubprotoState {
    pub fn new(config: &UpgradeSubprotoConfig) -> Self {
        let authorities = config
            .clone()
            .get_all_authorities()
            .into_iter()
            .map(|(role, config)| MultisigAuthority::new(role, config))
            .collect();

        Self {
            authorities,
            queued: Vec::new(),
            committed: Vec::new(),
            scheduled: Vec::new(),
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
    pub fn apply_multisig_update(&mut self, role: Role, update: &MultisigConfigUpdate) {
        if let Some(auth) = self.authority_mut(role) {
            auth.config_mut().apply(update);
        }
    }

    /// Find a queued update by its ID.
    pub fn find_queued(&self, id: &UpdateId) -> Option<&QueuedUpdate> {
        self.queued.iter().find(|u| u.id() == id)
    }

    /// Find a committed update by its ID.
    pub fn find_committed(&self, id: &UpdateId) -> Option<&CommittedUpdate> {
        self.committed.iter().find(|u| u.id() == id)
    }

    /// Find a scheduled update by its ID.
    pub fn find_scheduled(&self, id: &UpdateId) -> Option<&ScheduledUpdate> {
        self.scheduled.iter().find(|u| u.id() == id)
    }

    /// Queue a new update.
    pub fn enqueue(&mut self, update: QueuedUpdate) {
        self.queued.push(update);
    }

    /// Schedule an update to run without enactment.
    pub fn schedule(&mut self, update: ScheduledUpdate) {
        self.scheduled.push(update);
    }

    /// Remove a queued update by swapping it out.
    pub fn remove_queued(&mut self, id: &UpdateId) {
        if let Some(i) = self.queued.iter().position(|u| u.id() == id) {
            self.queued.swap_remove(i);
        }
    }

    /// Commit a scheduled update: move from committed to scheduled.
    pub fn commit_to_schedule(&mut self, id: &UpdateId, current_height: u64) {
        if let Some(i) = self.committed.iter().position(|u| u.id() == id) {
            let c_up = self.committed.swap_remove(i);
            let s_up = ScheduledUpdate::from_committed(c_up, current_height);
            self.scheduled.push(s_up);
        }
    }

    /// Get a reference to the next global update id
    pub fn next_update_id(&self) -> UpdateId {
        self.next_update_id
    }

    /// Increment the next global update id
    pub fn increment_next_update_id(&mut self) {
        self.next_update_id += 1;
    }

    /// Process all queued updates and move any whose `activation_height` equals `current_height`
    /// from `queued` into `committed`.
    pub fn process_queued(&mut self, current_height: u64) {
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queued)
            .into_iter()
            .partition(|u| u.activation_height() <= current_height);
        self.queued = rest;
        self.committed.extend(ready.into_iter().map(Into::into));
    }

    /// Process all queued updates and collect those whose `activation_height` equals
    /// `current_height`
    pub fn process_scheduled(&mut self, current_height: u64) -> Vec<ScheduledUpdate> {
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.scheduled)
            .into_iter()
            .partition(|u| u.activation_height() <= current_height);
        self.scheduled = rest;
        ready
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_proto_upgrade_txs::actions::{
        UpdateAction,
        updates::{multisig::MultisigUpdate, vk::VerifyingKeyUpdate},
    };
    use strata_crypto::multisig::{
        PubKey,
        config::{MultisigConfig, MultisigConfigUpdate},
    };
    use strata_primitives::roles::ProofType;
    use strata_test_utils::ArbitraryGenerator;
    use zkaleido::VerifyingKey;

    use crate::{
        state::{UpgradeSubprotoConfig, UpgradeSubprotoState},
        updates::{queued::QueuedUpdate, scheduled::ScheduledUpdate},
    };

    fn create_test_config() -> UpgradeSubprotoConfig {
        let test_key = PubKey::new([1; 32]);
        let test_config = MultisigConfig::try_new(vec![test_key], 1).unwrap();

        UpgradeSubprotoConfig::new(
            test_config.clone(),
            test_config.clone(),
            test_config.clone(),
            test_config,
        )
    }

    fn create_queued_action() -> UpdateAction {
        let vk_update = VerifyingKeyUpdate::new(VerifyingKey::default(), ProofType::OlStf);
        UpdateAction::VerifyingKey(vk_update)
    }

    fn create_scheduled_action() -> UpdateAction {
        let test_key = PubKey::new([2; 32]);
        let update = MultisigConfigUpdate::new(vec![test_key], vec![], 1);
        let multisig_update =
            MultisigUpdate::new(update, strata_primitives::roles::Role::BridgeAdmin);
        UpdateAction::Multisig(multisig_update)
    }

    #[test]
    fn test_enqueue_find_and_remove_queued() {
        let config = create_test_config();
        let mut state = UpgradeSubprotoState::new(&config);
        let mut arb = ArbitraryGenerator::new();

        let id = 1;
        let update: UpdateAction = arb.generate();
        let update = QueuedUpdate::try_new(id, update, 100).unwrap();
        state.enqueue(update.clone());

        assert_eq!(state.find_queued(&id), Some(&update));
        assert_eq!(state.find_queued(&2), None);

        state.remove_queued(&id);
        assert_eq!(state.find_queued(&id), None);
    }

    /// Helper to seed queued updates
    fn seed_queued(ids: &[u32], heights: &[u64]) -> UpgradeSubprotoState {
        let config = create_test_config();
        let mut state = UpgradeSubprotoState::new(&config);
        for (&id, &h) in ids.iter().zip(heights.iter()) {
            let action = create_queued_action();
            // Use current_height such that activation_height = h
            // Since OL_STF_VK_QUEUE_DELAY = 4320, current_height = h - 4320
            let current_height = if h >= 4320 { h - 4320 } else { 0 };
            state.enqueue(QueuedUpdate::try_new(id, action, current_height).unwrap());
        }
        state
    }

    /// Helper to seed scheduled updates
    fn seed_scheduled(
        ids: &[u32],
        heights: &[u64],
    ) -> (UpgradeSubprotoState, Vec<ScheduledUpdate>) {
        let mut arb = ArbitraryGenerator::new();
        let mut state = UpgradeSubprotoState::default();
        let mut updates = Vec::with_capacity(ids.len());
        for (&id, &h) in ids.iter().zip(heights.iter()) {
            let action: UpdateAction = arb.generate();
            let update = ScheduledUpdate::try_new(id, action, h).unwrap();
            state.schedule(update.clone());
            updates.push(update);
        }
        (state, updates)
    }

    #[test]
    fn test_process_queued_table() {
        struct Case {
            current: u64,
            want_q: Vec<u32>,
            want_c: Vec<u32>,
        }

        let ids = &[1, 2, 3];
        let heights = &[5000, 5100, 5200]; // Increased to work with delays

        let cases = vec![
            Case {
                current: 4999,
                want_q: vec![1, 2, 3],
                want_c: vec![],
            },
            Case {
                current: 5200,
                want_q: vec![],
                want_c: vec![1, 2, 3],
            },
            Case {
                current: 5100,
                want_q: vec![3],
                want_c: vec![1, 2],
            },
        ];

        for case in cases {
            let mut state = seed_queued(ids, heights);
            state.process_queued(case.current);

            let queued: Vec<_> = state.queued.iter().map(|u| *u.id()).collect();
            let mut committed: Vec<_> = state.committed.iter().map(|u| *u.id()).collect();
            committed.sort_unstable();

            assert_eq!(
                queued, case.want_q,
                "at height {} queued mismatch",
                case.current
            );
            assert_eq!(
                committed, case.want_c,
                "at height {} committed mismatch",
                case.current
            );
        }
    }

    #[test]
    fn test_process_scheduled_table() {
        struct Case {
            current: u64,
            want_rem: Vec<u32>,
            want_ret: Vec<u32>,
        }
        let ids = &[1, 2, 3];
        let heights = &[2500, 3000, 3500]; // Increased to work with delays

        let cases = vec![
            Case {
                current: 2499,
                want_rem: vec![1, 2, 3],
                want_ret: vec![],
            },
            Case {
                current: 2500,
                want_rem: vec![2, 3],
                want_ret: vec![1],
            },
            Case {
                current: 3000,
                want_rem: vec![3],
                want_ret: vec![1, 2],
            },
            Case {
                current: 3500,
                want_rem: vec![],
                want_ret: vec![1, 2, 3],
            },
        ];

        for case in cases {
            let (mut state, _) = seed_scheduled(ids, heights);
            let returned: Vec<_> = state
                .process_scheduled(case.current)
                .into_iter()
                .map(|u| *u.id())
                .collect();
            let mut remaining: Vec<_> = state.scheduled.iter().map(|u| *u.id()).collect();
            remaining.sort_unstable();

            assert_eq!(
                returned, case.want_ret,
                "at height {} returned mismatch",
                case.current
            );
            assert_eq!(
                remaining, case.want_rem,
                "at height {} remaining mismatch",
                case.current
            );
        }
    }

    #[test]
    fn test_commit_to_schedule() {
        let ids = &[1, 2, 3];
        let heights = &[5000, 5100, 5200]; // Increased to work with delays
        let mut state = seed_queued(ids, heights);

        state.process_queued(5200);
        assert_eq!(state.queued, &[]);
        assert_eq!(state.committed.len(), 3);
        assert_eq!(state.scheduled.len(), 0);

        state.commit_to_schedule(&2, 5300);

        // now committed should no longer contain 2, but still 1 & 3
        let mut remaining: Vec<_> = state.committed.iter().map(|c| *c.id()).collect();
        remaining.sort_unstable();
        assert_eq!(remaining, vec![1, 3]);

        // scheduled should now contain exactly 2
        let scheduled_ids: Vec<_> = state.scheduled.iter().map(|s| *s.id()).collect();
        assert_eq!(scheduled_ids, vec![2]);
    }
}
