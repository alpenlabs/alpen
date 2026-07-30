use std::{cmp::Ordering, collections::HashSet};

use strata_acct_types::Mmr64;
use strata_db_types::MmrId;
use strata_identifiers::AccountId;
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_merkle::MmrState;

use crate::OLMmrIndexError;

/// Caller-provided state for one persisted MMR index namespace.
#[derive(Clone, Debug)]
pub struct MmrIndexEntry {
    /// MMR namespace id.
    mmr_id: MmrId,

    /// Persisted MMR state.
    state: Mmr64,
}

impl MmrIndexEntry {
    /// Constructs an entry for a persisted MMR namespace.
    pub fn new(mmr_id: MmrId, state: Mmr64) -> Self {
        Self { mmr_id, state }
    }

    /// Constructs an entry for a namespace with no persisted leaves.
    pub fn empty(mmr_id: MmrId) -> Self {
        Self::new(mmr_id, Mmr64::new_empty())
    }

    /// Returns the MMR namespace id.
    pub fn mmr_id(&self) -> &MmrId {
        &self.mmr_id
    }

    /// Returns the persisted MMR state.
    pub fn state(&self) -> &Mmr64 {
        &self.state
    }

    /// Splits this entry into its owned parts.
    pub(crate) fn into_parts(self) -> (MmrId, Mmr64) {
        (self.mmr_id, self.state)
    }
}

/// An OL-owned MMR index with extra leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OLMmrIndexAhead {
    /// Divergent MMR namespace.
    mmr_id: MmrId,

    /// Persisted index leaf count.
    index_leaf_count: u64,

    /// Target state after truncation.
    target: Mmr64,
}

impl OLMmrIndexAhead {
    /// Returns the divergent MMR namespace.
    pub fn mmr_id(&self) -> &MmrId {
        &self.mmr_id
    }

    /// Returns the persisted index leaf count.
    pub fn index_leaf_count(&self) -> u64 {
        self.index_leaf_count
    }

    /// Returns the target MMR state after truncation.
    pub fn target(&self) -> &Mmr64 {
        &self.target
    }

    /// Splits this divergence into its owned parts.
    pub(crate) fn into_parts(self) -> (MmrId, u64, Mmr64) {
        (self.mmr_id, self.index_leaf_count, self.target)
    }
}

/// An OL-owned MMR index with fewer leaves than the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OLMmrIndexBehind {
    /// Divergent MMR namespace.
    mmr_id: MmrId,

    /// Persisted index leaf count.
    index_leaf_count: u64,

    /// Target leaf count.
    target_leaf_count: u64,
}

impl OLMmrIndexBehind {
    /// Returns the divergent MMR namespace.
    pub fn mmr_id(&self) -> &MmrId {
        &self.mmr_id
    }

    /// Returns the persisted index leaf count.
    pub fn index_leaf_count(&self) -> u64 {
        self.index_leaf_count
    }

    /// Returns the target leaf count.
    pub fn target_leaf_count(&self) -> u64 {
        self.target_leaf_count
    }
}

/// An OL-owned MMR index whose leaf count matches the target but whose MMR state
/// differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OLMmrIndexStateMismatch {
    /// Divergent MMR namespace.
    mmr_id: MmrId,

    /// Persisted index state.
    index_state: Mmr64,

    /// Target state.
    target: Mmr64,
}

impl OLMmrIndexStateMismatch {
    /// Returns the divergent MMR namespace.
    pub fn mmr_id(&self) -> &MmrId {
        &self.mmr_id
    }

    /// Returns the leaf count shared by the index and the target.
    ///
    /// The index and target agree on this count — that agreement is what makes
    /// this a state mismatch rather than an ahead/behind divergence.
    pub fn leaf_count(&self) -> u64 {
        self.index_state.num_entries()
    }

    /// Returns the persisted index state.
    pub fn index_state(&self) -> &Mmr64 {
        &self.index_state
    }

    /// Returns the target MMR state.
    pub fn target(&self) -> &Mmr64 {
        &self.target
    }
}

/// How an OL-owned MMR index differs from target state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OLMmrIndexDivergence {
    /// The index has extra leaves and can be truncated.
    Ahead(OLMmrIndexAhead),

    /// The index is missing leaves.
    Behind(OLMmrIndexBehind),

    /// The index has the target leaf count but a diverging state.
    StateMismatch(OLMmrIndexStateMismatch),
}

impl OLMmrIndexDivergence {
    /// Returns the divergent MMR namespace.
    pub fn mmr_id(&self) -> &MmrId {
        match self {
            Self::Ahead(divergence) => divergence.mmr_id(),
            Self::Behind(divergence) => divergence.mmr_id(),
            Self::StateMismatch(divergence) => divergence.mmr_id(),
        }
    }

    /// Returns the persisted index leaf count.
    pub fn index_leaf_count(&self) -> u64 {
        match self {
            Self::Ahead(divergence) => divergence.index_leaf_count(),
            Self::Behind(divergence) => divergence.index_leaf_count(),
            Self::StateMismatch(divergence) => divergence.leaf_count(),
        }
    }
}

/// One OL-owned MMR index that differs from target state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivergentOLMmrIndex {
    /// Divergence from target state.
    divergence: OLMmrIndexDivergence,
}

impl DivergentOLMmrIndex {
    /// Constructs a divergence when a persisted index differs from the target.
    fn from_index_entry(entry: MmrIndexEntry, target_mmr_state: Mmr64) -> Option<Self> {
        let (mmr_id, index_state) = entry.into_parts();
        let target_leaf_count = target_mmr_state.num_entries();
        let index_leaf_count = index_state.num_entries();
        match index_leaf_count.cmp(&target_leaf_count) {
            Ordering::Greater => Some(DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::Ahead(OLMmrIndexAhead {
                    mmr_id,
                    index_leaf_count,
                    target: target_mmr_state,
                }),
            }),
            // At equal leaf count the peak heights are fixed by the count, so
            // comparing the set peaks lowest-to-highest is exactly a peak-set match.
            Ordering::Equal if index_state.iter_peaks().eq(target_mmr_state.iter_peaks()) => None,
            Ordering::Equal => Some(DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::StateMismatch(OLMmrIndexStateMismatch {
                    mmr_id,
                    index_state,
                    target: target_mmr_state,
                }),
            }),
            Ordering::Less => Some(DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::Behind(OLMmrIndexBehind {
                    mmr_id,
                    index_leaf_count,
                    target_leaf_count,
                }),
            }),
        }
    }

    /// Returns the divergent MMR namespace.
    pub fn mmr_id(&self) -> &MmrId {
        self.divergence.mmr_id()
    }

    /// Returns the persisted index leaf count.
    pub fn index_leaf_count(&self) -> u64 {
        self.divergence.index_leaf_count()
    }

    /// Returns the divergence payload.
    pub fn divergence(&self) -> &OLMmrIndexDivergence {
        &self.divergence
    }

    /// Splits the wrapper into its divergence payload.
    pub(crate) fn into_divergence(self) -> OLMmrIndexDivergence {
        self.divergence
    }
}

/// Returns the OL target state for an MMR namespace.
///
/// Returns `None` for non-OL-owned namespaces such as [`MmrId::Asm`].
pub fn resolve_ol_mmr_target(
    target_state: &impl IStateAccessor,
    mmr_id: &MmrId,
) -> Result<Option<Mmr64>, OLMmrIndexError> {
    match mmr_id {
        MmrId::Asm => Ok(None),
        MmrId::L1BlockRefs => Ok(Some(target_state.l1_block_refs_mmr().clone())),
        MmrId::SnarkMsgInbox(account_id) => Ok(Some(resolve_target_snark_inbox_mmr(
            target_state,
            account_id,
        )?)),
    }
}

/// Returns OL-owned indexes that differ from target state.
///
/// Missing required OL-owned namespaces are treated as zero-count entries.
pub fn find_divergent_ol_mmr_indexes(
    target_state: &impl IStateAccessor,
    mut entries: Vec<MmrIndexEntry>,
    target_snark_accounts: impl IntoIterator<Item = AccountId>,
) -> Result<Vec<DivergentOLMmrIndex>, OLMmrIndexError> {
    let found_namespaces = entries
        .iter()
        .map(|entry| entry.mmr_id().clone())
        .collect::<HashSet<_>>();

    // Materialize missing OL namespaces as zero-leaf entries so comparison
    // reports `Behind` instead of silently skipping absent persisted rows.
    // `L1BlockRefs` is required whenever it is absent. Snark inbox namespaces
    // are required only when the target inbox has leaves.
    if !found_namespaces.contains(&MmrId::L1BlockRefs) {
        entries.push(MmrIndexEntry::empty(MmrId::L1BlockRefs));
    }

    for account_id in target_snark_accounts {
        let target = resolve_target_snark_inbox_mmr(target_state, &account_id)?;
        if target.num_entries() == 0 {
            continue;
        }

        let mmr_id = MmrId::SnarkMsgInbox(account_id);
        if !found_namespaces.contains(&mmr_id) {
            entries.push(MmrIndexEntry::empty(mmr_id));
        }
    }

    entries.sort_by_key(|entry| entry.mmr_id().to_bytes());

    let mut divergences = Vec::new();

    for entry in entries {
        let Some(target_mmr_state) = resolve_ol_mmr_target(target_state, entry.mmr_id())? else {
            continue;
        };

        if let Some(divergence) = DivergentOLMmrIndex::from_index_entry(entry, target_mmr_state) {
            divergences.push(divergence);
        }
    }

    Ok(divergences)
}

/// Returns the target inbox MMR state for a snark account.
fn resolve_target_snark_inbox_mmr(
    target_state: &impl IStateAccessor,
    account_id: &AccountId,
) -> Result<Mmr64, OLMmrIndexError> {
    let Some(snark_account) = target_state
        .get_account_state(*account_id)
        .map_err(|source| OLMmrIndexError::StateAccess {
            account_id: *account_id,
            source,
        })?
        .and_then(|account_state| account_state.as_snark_account().ok())
    else {
        return Ok(Mmr64::new_empty());
    };

    Ok(snark_account.inbox_mmr().clone())
}

#[cfg(test)]
mod tests {
    use strata_identifiers::{AccountId, Hash};

    use super::*;
    use crate::test_utils::{
        build_genesis_target_state, build_index_entry, build_repeated_leaf_mmr,
        build_snark_inbox_message, build_target_index_entry, build_target_state_accessor,
        build_target_state_with_snark_account, build_target_state_with_snark_inbox,
    };

    #[test]
    fn test_l1_refs_target_matches_epoch_mmr() {
        let state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&state);

        let target = resolve_ol_mmr_target(&target_accessor, &MmrId::L1BlockRefs)
            .expect("target MMR read should succeed")
            .expect("L1BlockRefs should be OL-owned");

        assert_eq!(target.num_entries(), 1);
        assert_eq!(&target, state.epoch_state().l1_block_refs_mmr());
    }

    #[test]
    fn test_asm_has_no_ol_target() {
        let state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&state);

        assert!(
            resolve_ol_mmr_target(&target_accessor, &MmrId::Asm)
                .expect("target MMR read should succeed")
                .is_none()
        );
    }

    #[test]
    fn test_matching_index_has_no_divergence() {
        let state = build_repeated_leaf_mmr(Hash::from([0x11; 32]), 2);
        let entry = MmrIndexEntry::new(MmrId::L1BlockRefs, state.clone());

        assert_eq!(DivergentOLMmrIndex::from_index_entry(entry, state), None);
    }

    #[test]
    fn test_matching_multi_peak_index_has_no_divergence() {
        // A leaf count of 3 has two peaks (heights 0 and 1), so equality must
        // agree across every peak — pinning `iter_peaks().eq` beyond one peak.
        let state = build_repeated_leaf_mmr(Hash::from([0x11; 32]), 3);
        let entry = MmrIndexEntry::new(MmrId::L1BlockRefs, state.clone());

        assert_eq!(DivergentOLMmrIndex::from_index_entry(entry, state), None);
    }

    #[test]
    fn test_longer_index_is_ahead() {
        let target = build_repeated_leaf_mmr(Hash::from([0x11; 32]), 2);
        let entry = MmrIndexEntry::new(
            MmrId::L1BlockRefs,
            build_repeated_leaf_mmr(Hash::from([0x11; 32]), 3),
        );

        assert_eq!(
            DivergentOLMmrIndex::from_index_entry(entry, target.clone()),
            Some(DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::Ahead(OLMmrIndexAhead {
                    mmr_id: MmrId::L1BlockRefs,
                    index_leaf_count: 3,
                    target,
                }),
            })
        );
    }

    #[test]
    fn test_shorter_index_is_behind() {
        let target = build_repeated_leaf_mmr(Hash::from([0x11; 32]), 2);
        let entry = MmrIndexEntry::new(
            MmrId::L1BlockRefs,
            build_repeated_leaf_mmr(Hash::from([0x11; 32]), 1),
        );

        assert_eq!(
            DivergentOLMmrIndex::from_index_entry(entry, target),
            Some(DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::Behind(OLMmrIndexBehind {
                    mmr_id: MmrId::L1BlockRefs,
                    index_leaf_count: 1,
                    target_leaf_count: 2,
                }),
            })
        );
    }

    #[test]
    fn test_diverging_state_is_mismatch() {
        let target = build_repeated_leaf_mmr(Hash::from([0x11; 32]), 2);
        let index_state = build_repeated_leaf_mmr(Hash::from([0x22; 32]), 2);
        let entry = MmrIndexEntry::new(MmrId::L1BlockRefs, index_state.clone());

        assert_eq!(
            DivergentOLMmrIndex::from_index_entry(entry, target.clone()),
            Some(DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::StateMismatch(OLMmrIndexStateMismatch {
                    mmr_id: MmrId::L1BlockRefs,
                    index_state,
                    target,
                }),
            })
        );
    }

    #[test]
    fn test_asm_is_omitted_from_divergences() {
        let target_state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&target_state);

        let divergences = find_divergent_ol_mmr_indexes(
            &target_accessor,
            vec![
                build_index_entry(MmrId::Asm, 3),
                build_target_index_entry(&target_state, MmrId::L1BlockRefs),
            ],
            target_state.iter_snark_account_ids(),
        )
        .expect("divergence scan should succeed");

        assert_eq!(divergences, Vec::new());
    }

    #[test]
    fn test_matching_index_is_omitted() {
        let target_state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&target_state);

        let divergences = find_divergent_ol_mmr_indexes(
            &target_accessor,
            vec![build_target_index_entry(&target_state, MmrId::L1BlockRefs)],
            target_state.iter_snark_account_ids(),
        )
        .expect("divergence scan should succeed");

        assert_eq!(divergences, Vec::new());
    }

    #[test]
    fn test_scan_reports_state_mismatch() {
        let target_state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&target_state);
        let target = resolve_ol_mmr_target(&target_accessor, &MmrId::L1BlockRefs)
            .expect("target MMR read should succeed")
            .expect("L1BlockRefs should be OL-owned");
        let index_state = build_repeated_leaf_mmr(Hash::from([0x42; 32]), target.num_entries());

        let divergences = find_divergent_ol_mmr_indexes(
            &target_accessor,
            vec![MmrIndexEntry::new(MmrId::L1BlockRefs, index_state.clone())],
            target_state.iter_snark_account_ids(),
        )
        .expect("divergence scan should succeed");

        assert_eq!(
            divergences,
            vec![DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::StateMismatch(OLMmrIndexStateMismatch {
                    mmr_id: MmrId::L1BlockRefs,
                    index_state,
                    target,
                }),
            }]
        );
    }

    #[test]
    fn test_missing_l1_refs_is_behind() {
        let target_state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&target_state);

        let divergences = find_divergent_ol_mmr_indexes(
            &target_accessor,
            Vec::new(),
            target_state.iter_snark_account_ids(),
        )
        .expect("divergence scan should succeed");

        assert_eq!(
            divergences,
            vec![DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::Behind(OLMmrIndexBehind {
                    mmr_id: MmrId::L1BlockRefs,
                    index_leaf_count: 0,
                    target_leaf_count: 1,
                }),
            }]
        );
    }

    #[test]
    fn test_missing_nonempty_inbox_is_behind() {
        let account_id = AccountId::new([0x44; 32]);
        let target_state =
            build_target_state_with_snark_inbox(account_id, vec![build_snark_inbox_message(0x55)]);
        let target_accessor = build_target_state_accessor(&target_state);

        let divergences = find_divergent_ol_mmr_indexes(
            &target_accessor,
            vec![build_target_index_entry(&target_state, MmrId::L1BlockRefs)],
            target_state.iter_snark_account_ids(),
        )
        .expect("divergence scan should succeed");

        assert_eq!(
            divergences,
            vec![DivergentOLMmrIndex {
                divergence: OLMmrIndexDivergence::Behind(OLMmrIndexBehind {
                    mmr_id: MmrId::SnarkMsgInbox(account_id),
                    index_leaf_count: 0,
                    target_leaf_count: 1,
                }),
            }]
        );
    }

    #[test]
    fn test_missing_empty_inbox_is_omitted() {
        let account_id = AccountId::new([0x44; 32]);
        let target_state = build_target_state_with_snark_account(account_id);
        let target_accessor = build_target_state_accessor(&target_state);

        let divergences = find_divergent_ol_mmr_indexes(
            &target_accessor,
            vec![build_target_index_entry(&target_state, MmrId::L1BlockRefs)],
            target_state.iter_snark_account_ids(),
        )
        .expect("divergence scan should succeed");

        assert_eq!(divergences, Vec::new());
    }

    #[test]
    fn test_divergences_sorted_by_mmr_id() {
        let target_state = build_genesis_target_state();
        let target_accessor = build_target_state_accessor(&target_state);
        let account_id = AccountId::new([0x77; 32]);
        let mut expected = vec![MmrId::L1BlockRefs, MmrId::SnarkMsgInbox(account_id)];
        expected.sort_by_key(MmrId::to_bytes);

        let divergences = find_divergent_ol_mmr_indexes(
            &target_accessor,
            vec![
                MmrIndexEntry::new(
                    MmrId::SnarkMsgInbox(account_id),
                    build_repeated_leaf_mmr(Hash::from([0x11; 32]), 1),
                ),
                MmrIndexEntry::new(
                    MmrId::L1BlockRefs,
                    build_repeated_leaf_mmr(Hash::from([0x11; 32]), 2),
                ),
            ],
            target_state.iter_snark_account_ids(),
        )
        .expect("divergence scan should succeed");

        assert_eq!(
            divergences
                .iter()
                .map(|divergent_index| divergent_index.mmr_id().clone())
                .collect::<Vec<_>>(),
            expected
        );
    }
}
