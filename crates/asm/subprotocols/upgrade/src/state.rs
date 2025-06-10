use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::{
    actions::UpdateId, crypto::multisig_config::MultisigConfigUpdate, roles::Role,
};

use crate::{
    authority::MultisigAuthority,
    upgrades::{committed::CommittedUpgrade, queued::QueuedUpgrade, scheduled::ScheduledUpgrade},
};

/// Holds the state for the upgrade subprotocol, including the various
/// multisignature authorities and any actions still pending execution.
#[derive(Debug, Clone, Eq, PartialEq, Default, BorshSerialize, BorshDeserialize)]
pub struct UpgradeSubprotoState {
    /// List of configurations for multisignature authorities.
    /// Each entry specifies who the signers are and how many signatures
    /// are required to approve an action.
    authorities: Vec<MultisigAuthority>,

    /// Actions that can still be cancelled by CancelTx while waiting
    /// for their block countdown to complete.
    queued: Vec<QueuedUpgrade>,

    /// Actions that have completed their waiting period and can be
    /// enacted by EnactmentTx, but can no longer be cancelled.
    committed: Vec<CommittedUpgrade>,

    /// Actions that will be executed automatically without requiring
    /// an EnactmentTx transaction.
    scheduled: Vec<ScheduledUpgrade>,

    /// UpdateId for the next update
    next_update_id: UpdateId,
}

impl UpgradeSubprotoState {
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

    /// Find a queued upgrade by its ID.
    pub fn find_queued(&self, id: &UpdateId) -> Option<&QueuedUpgrade> {
        self.queued.iter().find(|u| u.id() == id)
    }

    /// Find a committed upgrade by its ID.
    pub fn find_committed(&self, id: &UpdateId) -> Option<&CommittedUpgrade> {
        self.committed.iter().find(|u| u.id() == id)
    }

    /// Find a scheduled upgrade by its ID.
    pub fn find_scheduled(&self, id: &UpdateId) -> Option<&ScheduledUpgrade> {
        self.scheduled.iter().find(|u| u.id() == id)
    }

    /// Queue a new upgrade.
    pub fn enqueue(&mut self, upgrade: QueuedUpgrade) {
        self.queued.push(upgrade);
    }

    /// Schedule an upgrade to run without enactment.
    pub fn schedule(&mut self, upgrade: ScheduledUpgrade) {
        self.scheduled.push(upgrade);
    }

    /// Remove a queued upgrade by swapping it out.
    pub fn remove_queued(&mut self, id: &UpdateId) {
        if let Some(i) = self.queued.iter().position(|u| u.id() == id) {
            self.queued.swap_remove(i);
        }
    }

    /// Commit a scheduled upgrade: move from committed to scheduled.
    pub fn commit_to_schedule(&mut self, id: &UpdateId) {
        if let Some(i) = self.committed.iter().position(|u| u.id() == id) {
            let up = self.committed.swap_remove(i);
            self.scheduled.push(up.into());
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

    /// Process all queued upgrades and move any whose `activation_height` equals `current_height`
    /// from `queued` into `committed`.
    pub fn process_queued(&mut self, current_height: u64) {
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queued)
            .into_iter()
            .partition(|u| u.activation_height() <= current_height);
        self.queued = rest;
        self.committed.extend(ready.into_iter().map(Into::into));
    }

    /// Process all queued upgrades and collect those whose `activation_height` equals
    /// `current_height`
    pub fn process_scheduled(&mut self, current_height: u64) -> Vec<ScheduledUpgrade> {
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.scheduled)
            .into_iter()
            .partition(|u| u.activation_height() <= current_height);
        self.scheduled = rest;
        ready
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_proto_upgrade_txs::actions::UpgradeAction;
    use strata_test_utils::ArbitraryGenerator;

    use crate::{
        state::UpgradeSubprotoState,
        upgrades::{queued::QueuedUpgrade, scheduled::ScheduledUpgrade},
    };

    #[test]
    fn test_enqueue_find_and_remove_queued() {
        let mut arb = ArbitraryGenerator::new();
        let mut state = UpgradeSubprotoState::default();

        let id = 1;
        let update: UpgradeAction = arb.generate();
        let upgrade = QueuedUpgrade::new(id, update, 100);
        state.enqueue(upgrade.clone());

        assert_eq!(state.find_queued(&id), Some(&upgrade));
        assert_eq!(state.find_queued(&2), None);

        state.remove_queued(&id);
        assert_eq!(state.find_queued(&id), None);
    }

    /// Helper to seed queued upgrades
    fn seed_queued(ids: &[u32], heights: &[u64]) -> UpgradeSubprotoState {
        let mut arb = ArbitraryGenerator::new();
        let mut state = UpgradeSubprotoState::default();
        for (&id, &h) in ids.iter().zip(heights.iter()) {
            let action: UpgradeAction = arb.generate();
            state.enqueue(QueuedUpgrade::new(id, action, h));
        }
        state
    }

    /// Helper to seed scheduled upgrades
    fn seed_scheduled(
        ids: &[u32],
        heights: &[u64],
    ) -> (UpgradeSubprotoState, Vec<ScheduledUpgrade>) {
        let mut arb = ArbitraryGenerator::new();
        let mut state = UpgradeSubprotoState::default();
        let mut upgrades = Vec::with_capacity(ids.len());
        for (&id, &h) in ids.iter().zip(heights.iter()) {
            let action: UpgradeAction = arb.generate();
            let upgrade = ScheduledUpgrade::new(id, action, h);
            state.schedule(upgrade.clone());
            upgrades.push(upgrade);
        }
        (state, upgrades)
    }

    #[test]
    fn test_process_queued_table() {
        struct Case {
            current: u64,
            want_q: Vec<u32>,
            want_c: Vec<u32>,
        }

        let ids = &[1, 2, 3];
        let heights = &[5, 10, 15];

        let cases = vec![
            Case {
                current: 4,
                want_q: vec![1, 2, 3],
                want_c: vec![],
            },
            Case {
                current: 20,
                want_q: vec![],
                want_c: vec![1, 2, 3],
            },
            Case {
                current: 10,
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
        let heights = &[5, 10, 15];

        let cases = vec![
            Case {
                current: 4,
                want_rem: vec![1, 2, 3],
                want_ret: vec![],
            },
            Case {
                current: 5,
                want_rem: vec![2, 3],
                want_ret: vec![1],
            },
            Case {
                current: 10,
                want_rem: vec![3],
                want_ret: vec![1, 2],
            },
            Case {
                current: 15,
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
        let heights = &[5, 10, 15];
        let mut state = seed_queued(ids, heights);

        state.process_queued(15);
        assert_eq!(state.queued, &[]);
        assert_eq!(state.committed.len(), 3);
        assert_eq!(state.scheduled.len(), 0);

        state.commit_to_schedule(&2);

        // now committed should no longer contain 2, but still 1 & 3
        let mut remaining: Vec<_> = state.committed.iter().map(|c| *c.id()).collect();
        remaining.sort_unstable();
        assert_eq!(remaining, vec![1, 3]);

        // scheduled should now contain exactly 2
        let scheduled_ids: Vec<_> = state.scheduled.iter().map(|s| *s.id()).collect();
        assert_eq!(scheduled_ids, vec![2]);
    }
}
