use strata_acct_types::Mmr64;
use strata_db_types::MmrId;
use strata_identifiers::AccountId;
use strata_ledger_types::IStateAccessor;

use crate::{
    MmrIndexEntry, OLMmrIndexAhead, OLMmrIndexDivergence, OLMmrIndexError,
    find_divergent_ol_mmr_indexes,
};

/// One persisted MMR index namespace that can be truncated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MmrIndexTruncation {
    mmr_id: MmrId,
    index_leaf_count: u64,
    target: Mmr64,
}

impl MmrIndexTruncation {
    /// Returns the MMR namespace to truncate.
    pub fn mmr_id(&self) -> &MmrId {
        &self.mmr_id
    }

    /// Returns the current persisted index leaf count.
    pub fn index_leaf_count(&self) -> u64 {
        self.index_leaf_count
    }

    /// Returns the target MMR state after truncation.
    pub fn target(&self) -> &Mmr64 {
        &self.target
    }

    /// Returns how many leaves the truncation removes.
    ///
    /// Truncations are constructed only for ahead indexes, so the index count
    /// is always greater than the target count.
    pub fn leaves_to_remove(&self) -> u64 {
        self.index_leaf_count - self.target.num_entries()
    }
}

impl From<OLMmrIndexAhead> for MmrIndexTruncation {
    fn from(ahead_index: OLMmrIndexAhead) -> Self {
        let (mmr_id, index_leaf_count, target) = ahead_index.into_parts();
        Self {
            mmr_id,
            index_leaf_count,
            target,
        }
    }
}

/// Summary of one MMR index reconciliation plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MmrIndexReconcileReport {
    /// Number of persisted MMR namespaces inspected.
    pub inspected: u64,

    /// Number of ASM-owned namespaces skipped.
    pub asm_owned_skipped: u64,

    /// Number of MMR namespaces to truncate.
    pub indexes_truncated: u64,

    /// Number of leaves to remove across all truncations.
    pub leaves_removed: u64,
}

/// Validated plan for reconciling a persisted MMR index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MmrIndexReconcilePlan {
    inspected: u64,
    asm_owned_skipped: u64,
    truncations: Vec<MmrIndexTruncation>,
}

impl MmrIndexReconcilePlan {
    /// Constructs a plan from already validated parts.
    pub(crate) fn new(
        inspected: u64,
        asm_owned_skipped: u64,
        truncations: Vec<MmrIndexTruncation>,
    ) -> Self {
        debug_assert!(
            inspected >= asm_owned_skipped + u64::try_from(truncations.len()).unwrap_or(u64::MAX),
            "inspected count must cover skipped and truncated namespaces"
        );
        Self {
            inspected,
            asm_owned_skipped,
            truncations,
        }
    }

    /// Returns the inspected namespace count.
    pub fn inspected(&self) -> u64 {
        self.inspected
    }

    /// Returns the skipped ASM-owned namespace count.
    pub fn asm_owned_skipped(&self) -> u64 {
        self.asm_owned_skipped
    }

    /// Returns persisted MMR indexes to truncate.
    pub fn truncations(&self) -> &[MmrIndexTruncation] {
        &self.truncations
    }

    /// Returns the truncation count.
    pub fn truncation_count(&self) -> u64 {
        u64::try_from(self.truncations.len()).expect("MMR truncate count should fit in u64")
    }

    /// Returns the removable leaf count.
    pub fn leaves_to_remove_count(&self) -> u64 {
        self.truncations
            .iter()
            .map(MmrIndexTruncation::leaves_to_remove)
            .sum()
    }

    /// Returns an operator-facing summary of the plan.
    pub fn to_report(&self) -> MmrIndexReconcileReport {
        MmrIndexReconcileReport {
            inspected: self.inspected(),
            asm_owned_skipped: self.asm_owned_skipped(),
            indexes_truncated: self.truncation_count(),
            leaves_removed: self.leaves_to_remove_count(),
        }
    }
}

/// Builds a validated plan from persisted MMR index entries and target state.
pub fn build_mmr_index_reconcile_plan(
    target_state: &impl IStateAccessor,
    entries: Vec<MmrIndexEntry>,
    target_snark_accounts: impl IntoIterator<Item = AccountId>,
) -> Result<MmrIndexReconcilePlan, OLMmrIndexError> {
    let l1_block_refs_target_leaf_count = target_state.l1_block_refs_mmr().num_entries();
    if l1_block_refs_target_leaf_count == 0 {
        return Err(OLMmrIndexError::L1BlockRefsMissingSentinel);
    }

    let inspected_entries = entries.len();
    let asm_owned_skipped = entries
        .iter()
        .filter(|entry| matches!(entry.mmr_id(), MmrId::Asm))
        .count();
    let mut truncations = Vec::new();
    for divergent_index in
        find_divergent_ol_mmr_indexes(target_state, entries, target_snark_accounts)?
    {
        match divergent_index.into_divergence() {
            OLMmrIndexDivergence::Ahead(ahead_index) => {
                truncations.push(MmrIndexTruncation::from(ahead_index));
            }
            OLMmrIndexDivergence::Behind(behind_index) => {
                return Err(OLMmrIndexError::BehindTarget {
                    mmr_id: behind_index.mmr_id().clone(),
                    index_leaf_count: behind_index.index_leaf_count(),
                    target_leaf_count: behind_index.target_leaf_count(),
                });
            }
            OLMmrIndexDivergence::StateMismatch(mismatch) => {
                return Err(OLMmrIndexError::StateMismatch {
                    mmr_id: mismatch.mmr_id().clone(),
                    leaf_count: mismatch.leaf_count(),
                });
            }
        }
    }

    Ok(MmrIndexReconcilePlan::new(
        u64::try_from(inspected_entries).expect("MMR inspection count should fit in u64"),
        u64::try_from(asm_owned_skipped).expect("MMR skipped count should fit in u64"),
        truncations,
    ))
}

#[cfg(test)]
mod tests {
    use strata_db_types::MmrId;
    use strata_identifiers::{AccountId, Hash};

    use super::*;
    use crate::{
        MmrIndexEntry, OLMmrIndexError, resolve_ol_mmr_target,
        test_utils::{
            build_genesis_target_state, build_index_entry, build_repeated_leaf_mmr,
            build_snark_inbox_message, build_target_index_entry, build_target_state_accessor,
            build_target_state_with_empty_l1_block_refs_mmr, build_target_state_with_snark_inbox,
        },
    };

    #[test]
    fn test_plan_counts_namespaces() {
        let account_id = AccountId::new([0x44; 32]);
        let target_state =
            build_target_state_with_snark_inbox(account_id, vec![build_snark_inbox_message(0x55)]);
        let target_accessor = build_target_state_accessor(&target_state);

        let plan = build_mmr_index_reconcile_plan(
            &target_accessor,
            vec![
                build_index_entry(MmrId::Asm, 3),
                build_index_entry(MmrId::L1BlockRefs, 3),
                build_target_index_entry(&target_state, MmrId::SnarkMsgInbox(account_id)),
            ],
            target_state.iter_snark_account_ids(),
        )
        .expect("valid plan");

        assert_eq!(plan.inspected(), 3);
        assert_eq!(plan.asm_owned_skipped(), 1);
        assert_eq!(plan.truncations().len(), 1);
        let truncation = plan.truncations().first().expect("truncation");
        assert_eq!(truncation.mmr_id(), &MmrId::L1BlockRefs);
        assert_eq!(truncation.index_leaf_count(), 3);
        assert_eq!(truncation.target().num_entries(), 1);
        assert_eq!(truncation.leaves_to_remove(), 2);
        assert_eq!(plan.truncation_count(), 1);
        assert_eq!(plan.leaves_to_remove_count(), 2);
        assert_eq!(
            plan.to_report(),
            MmrIndexReconcileReport {
                inspected: 3,
                asm_owned_skipped: 1,
                indexes_truncated: 1,
                leaves_removed: 2,
            }
        );
    }

    #[test]
    fn test_plan_rejects_behind_target() {
        let target_state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&target_state);

        let err = build_mmr_index_reconcile_plan(
            &target_accessor,
            Vec::new(),
            target_state.iter_snark_account_ids(),
        )
        .expect_err("behind target should fail");

        assert!(
            matches!(
                &err,
                OLMmrIndexError::BehindTarget {
                    mmr_id,
                    index_leaf_count: 0,
                    target_leaf_count: 1,
                } if mmr_id == &MmrId::L1BlockRefs
            ),
            "unexpected error: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "MMR l1-block-refs is behind target (index leaf count 0, target leaf count 1)"
        );
    }

    #[test]
    fn test_plan_rejects_state_mismatch() {
        let target_state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&target_state);
        let target = resolve_ol_mmr_target(&target_accessor, &MmrId::L1BlockRefs)
            .expect("target MMR read should succeed")
            .expect("L1BlockRefs should be OL-owned");

        let err = build_mmr_index_reconcile_plan(
            &target_accessor,
            vec![MmrIndexEntry::new(
                MmrId::L1BlockRefs,
                build_repeated_leaf_mmr(Hash::from([0x42; 32]), target.num_entries()),
            )],
            target_state.iter_snark_account_ids(),
        )
        .expect_err("state mismatch should fail");

        assert!(
            matches!(
                &err,
                OLMmrIndexError::StateMismatch {
                    mmr_id,
                    leaf_count: 1,
                } if mmr_id == &MmrId::L1BlockRefs
            ),
            "unexpected error: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "MMR l1-block-refs state does not match target at leaf count 1"
        );
    }

    #[test]
    fn test_plan_rejects_missing_sentinel() {
        let target_state = build_target_state_with_empty_l1_block_refs_mmr();
        let target_accessor = build_target_state_accessor(&target_state);

        let err = build_mmr_index_reconcile_plan(
            &target_accessor,
            vec![build_index_entry(MmrId::L1BlockRefs, 0)],
            target_state.iter_snark_account_ids(),
        )
        .expect_err("sentinel floor should fail");

        assert!(matches!(&err, OLMmrIndexError::L1BlockRefsMissingSentinel));
        assert_eq!(
            err.to_string(),
            "MMR l1-block-refs target is missing the genesis sentinel"
        );
    }
}
