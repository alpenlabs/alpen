use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Display},
    str::FromStr,
};

use argh::FromArgs;
use ssz::Decode;
use strata_acct_types::{L1BlockRecord, MessageEntry, Mmr64};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_crypto::hash;
use strata_db_types::{
    backend::DatabaseBackend, mmr_index::MmrIndexDatabase, num_leaves_to_mmr_size, LeafPos, MmrId,
    RawMmrId,
};
use strata_identifiers::{AccountId, Hash};
use strata_ledger_types::{IAccountState, ISnarkAccountState};
use strata_ol_state_types::{OLState, MMR_SENTINEL_DUMMY_LEAF_HASH};
use strata_storage::{MmrIndexManager, MmrStateView};
use tokio::runtime::Runtime;

use crate::{
    cli::OutputFormat,
    output::{
        mmr::{MmrLeafInfo, MmrOwner, MmrPreimageDecoded, MmrSummaryEntry, MmrSummaryInfo},
        output,
    },
};

const MMR_ID_ASM: &str = "asm";
const MMR_ID_L1_BLOCK_REFS: &str = "l1-block-refs";
const MMR_ID_SNARK_MSG_INBOX_PREFIX: &str = "snark-msg-inbox";

/// Shows indexed MMR namespaces and their leaf counts.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-mmr-summary")]
pub(crate) struct GetMmrSummaryArgs {
    /// optional owner filter: "asm" or "ol"
    #[argh(option)]
    pub(crate) owner: Option<MmrOwner>,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Shows one indexed MMR leaf.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-mmr-leaf")]
pub(crate) struct GetMmrLeafArgs {
    /// MMR id: `asm`, `l1-block-refs`, or `snark-msg-inbox:<account-hex>`
    #[argh(positional)]
    pub(crate) mmr_id: MmrIdInput,

    /// leaf index to read
    #[argh(positional)]
    pub(crate) leaf_index: u64,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnsupportedMmrId(String);

impl UnsupportedMmrId {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn unsupported_id() -> Self {
        Self::new("must be 'asm', 'l1-block-refs', or 'snark-msg-inbox:<account-hex>'")
    }
}

impl Display for UnsupportedMmrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MmrNamespace {
    mmr_id: MmrId,
}

impl MmrNamespace {
    pub(crate) fn new(mmr_id: MmrId) -> Self {
        Self { mmr_id }
    }

    pub(crate) fn from_raw_mmr_id(raw_mmr_id: &RawMmrId) -> Result<Self, DisplayedError> {
        let raw_mmr_id_hex = hex::encode(raw_mmr_id);
        let mmr_id = MmrId::from_bytes(raw_mmr_id).map_err(|e| {
            DisplayedError::InternalError(
                format!("MMR namespace id {raw_mmr_id_hex} is not a known MmrId: {e}"),
                Box::new(raw_mmr_id_hex.clone()),
            )
        })?;

        Ok(Self::new(mmr_id))
    }

    fn from_cli_input(input: &str) -> Result<Self, UnsupportedMmrId> {
        let normalized_input = normalize_mmr_id_part(input);
        match normalized_input.as_str() {
            MMR_ID_ASM => Ok(Self::new(MmrId::Asm)),
            MMR_ID_L1_BLOCK_REFS => Ok(Self::new(MmrId::L1BlockRefs)),
            _ => {
                let (prefix, account_hex) = normalized_input
                    .split_once(':')
                    .ok_or_else(UnsupportedMmrId::unsupported_id)?;
                match prefix {
                    MMR_ID_SNARK_MSG_INBOX_PREFIX => Ok(Self::new(MmrId::SnarkMsgInbox(
                        parse_account_id(account_hex)?,
                    ))),
                    _ => Err(UnsupportedMmrId::unsupported_id()),
                }
            }
        }
    }

    pub(crate) fn as_mmr_id(&self) -> &MmrId {
        &self.mmr_id
    }

    pub(crate) fn raw_mmr_id(&self) -> RawMmrId {
        self.as_mmr_id().to_bytes()
    }

    pub(crate) fn display_id(&self) -> String {
        match &self.mmr_id {
            MmrId::Asm => MMR_ID_ASM.to_string(),
            MmrId::L1BlockRefs => MMR_ID_L1_BLOCK_REFS.to_string(),
            MmrId::SnarkMsgInbox(account_id) => {
                format!("{MMR_ID_SNARK_MSG_INBOX_PREFIX}:{account_id}")
            }
        }
    }

    pub(crate) fn owner(&self) -> MmrOwner {
        match &self.mmr_id {
            MmrId::Asm => MmrOwner::Asm,
            MmrId::L1BlockRefs | MmrId::SnarkMsgInbox(_) => MmrOwner::OL,
        }
    }

    pub(crate) fn account(&self) -> Option<String> {
        match &self.mmr_id {
            MmrId::SnarkMsgInbox(account_id) => Some(account_id.to_string()),
            MmrId::Asm | MmrId::L1BlockRefs => None,
        }
    }

    fn ol_target_view(&self, target_state: &OLState) -> Option<MmrStateView> {
        match &self.mmr_id {
            MmrId::Asm => None,
            MmrId::L1BlockRefs => Some(get_mmr_state_view(
                target_state.epoch_state().l1_block_refs_mmr(),
            )),
            MmrId::SnarkMsgInbox(account_id) => {
                Some(get_target_snark_inbox_mmr_state(target_state, account_id))
            }
        }
    }

    fn expects_typed_preimage(&self) -> bool {
        matches!(&self.mmr_id, MmrId::L1BlockRefs | MmrId::SnarkMsgInbox(_))
    }

    fn is_sentinel_dummy_leaf(&self, leaf_hash: &Hash) -> bool {
        matches!(&self.mmr_id, MmrId::Asm | MmrId::L1BlockRefs)
            && *leaf_hash == MMR_SENTINEL_DUMMY_LEAF_HASH
    }

    fn decode_preimage(&self, leaf_index: u64, preimage: &[u8]) -> Result<DecodedPreimage, String> {
        match &self.mmr_id {
            MmrId::Asm => Err("ASM MMR leaves do not have a typed preimage format".to_string()),
            MmrId::L1BlockRefs => {
                let record = L1BlockRecord::from_ssz_bytes(preimage)
                    .map_err(|e| format!("failed to decode L1 block ref preimage: {e:?}"))?;
                Ok(DecodedPreimage {
                    expected_leaf_hash: Hash::from(record.leaf_hash()),
                    preimage_decoded: MmrPreimageDecoded::L1BlockRef {
                        height: leaf_index,
                        block_hash: hex::encode(record.block_hash()),
                        wtxids_root: hex::encode(record.wtxids_root()),
                    },
                })
            }
            MmrId::SnarkMsgInbox(_) => {
                let message = MessageEntry::from_ssz_bytes(preimage)
                    .map_err(|e| format!("failed to decode snark inbox preimage: {e:?}"))?;
                let payload = message.payload_buf();
                Ok(DecodedPreimage {
                    expected_leaf_hash: message.compute_msg_commitment(),
                    preimage_decoded: MmrPreimageDecoded::SnarkMsgInbox {
                        source: message.source().to_string(),
                        inclusion_epoch: message.incl_epoch(),
                        payload_len: u64::try_from(payload.len())
                            .expect("payload length should fit in u64"),
                        payload_hash: hash_to_hex(&hash::raw(payload)),
                    },
                })
            }
        }
    }
}

impl Display for MmrNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_id())
    }
}

/// Normalizes CLI input to the canonical dash-separated MMR id form.
///
/// The command prints dash ids for copy-paste use, but accepts snake_case input
/// because those names mirror Rust enum variants and are easy to type.
fn normalize_mmr_id_part(input: &str) -> String {
    input.replace('_', "-")
}

fn display_mmr_id(mmr_id: &MmrId) -> String {
    MmrNamespace::new(mmr_id.clone()).display_id()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MmrIdInput(MmrNamespace);

impl MmrIdInput {
    fn namespace(&self) -> &MmrNamespace {
        &self.0
    }
}

impl Display for MmrIdInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for MmrIdInput {
    type Err = UnsupportedMmrId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        MmrNamespace::from_cli_input(s).map(Self)
    }
}

/// Records one MMR namespace enumerated from the index database.
#[derive(Clone, Debug)]
struct MmrNamespaceRecord {
    raw_mmr_id: RawMmrId,
    namespace: MmrNamespace,
    leaf_count: u64,
}

/// Carries one fetched MMR leaf and its optional preimage.
#[derive(Debug)]
struct MmrLeafData {
    raw_mmr_id: RawMmrId,
    leaf_index: u64,
    leaf_count: u64,
    leaf_hash: Hash,
    preimage: Option<Vec<u8>>,
}

/// Holds a decoded MMR preimage and the leaf hash it should produce.
#[derive(Debug)]
struct DecodedPreimage {
    expected_leaf_hash: Hash,
    preimage_decoded: MmrPreimageDecoded,
}

/// One MMR index that is a candidate to be reverted (popped back) to the target OL state.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MmrIndexRevertCandidate {
    mmr_id: MmrId,
    current_leaf_count: u64,
    target_leaf_count: u64,
    target_peaks: Vec<Hash>,
}

impl MmrIndexRevertCandidate {
    /// Returns the number of leaves to remove from this MMR.
    fn pop_count(&self) -> u64 {
        self.current_leaf_count - self.target_leaf_count
    }
}

/// Describes one MMR whose persisted count is below the target OL state.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MmrIndexBehindTarget {
    mmr_id: MmrId,
    current_leaf_count: u64,
    target_leaf_count: u64,
}

/// Describes a post-pop leaf-count mismatch for one MMR.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MmrIndexFinalLeafCountMismatch {
    revert_candidate: MmrIndexRevertCandidate,
    final_leaf_count: u64,
}

/// Describes a post-pop peak mismatch for one MMR.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MmrIndexFinalPeaksMismatch {
    revert_candidate: MmrIndexRevertCandidate,
    final_peaks: Vec<Hash>,
}

/// Describes all MMR changes needed for one `revert-ol-state` run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MmrIndexRevertPlan {
    inspected: u64,
    asm_owned_skipped: u64,
    revert_candidates: Vec<MmrIndexRevertCandidate>,
    behind_target: Vec<MmrIndexBehindTarget>,
}

impl MmrIndexRevertPlan {
    /// Returns the number of MMRs with pending pops.
    fn mmrs_to_revert(&self) -> u64 {
        u64::try_from(self.revert_candidates.len()).expect("MMR revert count should fit in u64")
    }

    /// Returns the total number of leaves to pop across all MMRs.
    fn leaves_to_pop(&self) -> u64 {
        self.revert_candidates
            .iter()
            .map(MmrIndexRevertCandidate::pop_count)
            .sum()
    }
}

/// Shows indexed MMR namespaces and their basic stats.
pub(crate) fn get_mmr_summary(
    db: &impl DatabaseBackend,
    args: GetMmrSummaryArgs,
) -> Result<(), DisplayedError> {
    let records = get_mmr_namespace_records(db)?;
    let summary = build_mmr_summary_info(records, args.owner);

    output(&summary, args.output_format)
}

/// Shows one indexed MMR leaf.
pub(crate) fn get_mmr_leaf(
    db: &impl DatabaseBackend,
    args: GetMmrLeafArgs,
) -> Result<(), DisplayedError> {
    let leaf_data = get_mmr_leaf_data(db, args.mmr_id.namespace(), args.leaf_index)?;
    let leaf_info = build_mmr_leaf_info(args.mmr_id.namespace(), leaf_data)?;

    output(&leaf_info, args.output_format)
}

/// Parses a user-provided account id as 32 bytes of hex.
fn parse_account_id(account_hex: &str) -> Result<AccountId, UnsupportedMmrId> {
    let trimmed = account_hex.strip_prefix("0x").unwrap_or(account_hex);
    let bytes = hex::decode(trimmed)
        .map_err(|e| UnsupportedMmrId::new(format!("invalid account hex: {e}")))?;
    let bytes: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
        UnsupportedMmrId::new(format!(
            "account id must be exactly 32 bytes (got {} bytes)",
            bytes.len()
        ))
    })?;

    Ok(AccountId::new(bytes))
}

/// Reads all MMR namespaces visible through the persisted leaf-count index.
///
/// The command treats an undecodable namespace id as corruption and returns an
/// error instead of skipping it. Empty namespaces with a `leaf_count: 0` row are
/// included so callers can decide whether to display or roll them back.
fn get_mmr_namespace_records(
    db: &impl DatabaseBackend,
) -> Result<Vec<MmrNamespaceRecord>, DisplayedError> {
    let mmr_db = db.mmr_index_db();
    let mut records = Vec::new();

    for raw_mmr_id in mmr_db
        .list_mmr_ids()
        .internal_error("Failed to list MMR ids")?
    {
        let leaf_count = mmr_db
            .get_leaf_count(raw_mmr_id.clone())
            .internal_error("Failed to read MMR leaf count")?;
        let namespace = MmrNamespace::from_raw_mmr_id(&raw_mmr_id)?;

        records.push(MmrNamespaceRecord {
            raw_mmr_id,
            namespace,
            leaf_count,
        });
    }

    Ok(records)
}

/// Reads one MMR leaf and its optional preimage from the index database.
///
/// The function reports empty namespaces and out-of-range indexes as user
/// errors. A missing leaf node inside the advertised range is an internal
/// consistency error.
fn get_mmr_leaf_data(
    db: &impl DatabaseBackend,
    namespace: &MmrNamespace,
    leaf_index: u64,
) -> Result<MmrLeafData, DisplayedError> {
    let mmr_db = db.mmr_index_db();
    let raw_mmr_id = namespace.raw_mmr_id();
    let leaf_pos = LeafPos::new(leaf_index);
    let query_id = namespace.display_id();

    let leaf_count = mmr_db
        .get_leaf_count(raw_mmr_id.clone())
        .internal_error("Failed to read MMR leaf count")?;
    if leaf_count == 0 {
        return Err(DisplayedError::UserError(
            format!("MMR {query_id} is not found or empty"),
            Box::new(query_id),
        ));
    }
    if leaf_index >= leaf_count {
        return Err(DisplayedError::UserError(
            format!("MMR leaf index {leaf_index} is out of range for leaf count {leaf_count}"),
            Box::new(leaf_index),
        ));
    }

    let leaf_hash = mmr_db
        .get_node(raw_mmr_id.clone(), leaf_pos.to_node_pos())
        .internal_error("Failed to read MMR leaf hash")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                format!("MMR leaf hash is missing for {query_id} at index {leaf_index}"),
                Box::new(leaf_index),
            )
        })?;
    let preimage = mmr_db
        .get_preimage(raw_mmr_id.clone(), leaf_pos)
        .internal_error("Failed to read MMR leaf preimage")?;

    Ok(MmrLeafData {
        raw_mmr_id,
        leaf_index,
        leaf_count,
        leaf_hash,
        preimage,
    })
}

/// Builds the MMR revert plan for a target OL state.
///
/// The plan compares the persisted index-DB MMRs against the in-state MMRs
/// kept by `target_state`. It performs only reads; validation and mutation are
/// separate steps.
pub(crate) fn get_mmr_index_revert_plan(
    db: &impl DatabaseBackend,
    target_state: &OLState,
) -> Result<MmrIndexRevertPlan, DisplayedError> {
    let records = get_mmr_namespace_records(db)?;
    Ok(build_mmr_index_revert_plan(target_state, records))
}

/// Classifies each MMR namespace as skipped, already aligned, ahead, or behind.
///
/// ASM-owned MMRs are skipped because `revert-ol-state` only reverts OL-owned
/// MMR entries. `L1BlockRefs` and `SnarkMsgInbox` MMRs are compared
/// against the target OL state's compact MMRs.
fn build_mmr_index_revert_plan(
    target_state: &OLState,
    mut records: Vec<MmrNamespaceRecord>,
) -> MmrIndexRevertPlan {
    records.sort_by(|a, b| a.raw_mmr_id.cmp(&b.raw_mmr_id));

    let mut plan = MmrIndexRevertPlan {
        inspected: u64::try_from(records.len()).expect("MMR record count should fit in u64"),
        ..MmrIndexRevertPlan::default()
    };
    let mut found_namespaces = HashSet::new();

    for record in records {
        found_namespaces.insert(record.namespace.as_mmr_id().clone());
        if let Some(target_mmr_state) = record.namespace.ol_target_view(target_state) {
            add_mmr_target_to_plan(&mut plan, record, target_mmr_state);
        } else {
            plan.asm_owned_skipped += 1;
        }
    }

    add_missing_target_ol_namespaces(&mut plan, target_state, &found_namespaces);

    plan
}

/// Adds target OL namespaces that are required by state but missing in the MMR index DB.
///
/// Missing namespaces are treated as persisted leaf count zero. This catches
/// index DBs that are behind a non-empty target state even when no leaf-count row
/// exists for the namespace.
fn add_missing_target_ol_namespaces(
    plan: &mut MmrIndexRevertPlan,
    target_state: &OLState,
    found_namespaces: &HashSet<MmrId>,
) {
    if !found_namespaces.contains(&MmrId::L1BlockRefs) {
        let namespace = MmrNamespace::new(MmrId::L1BlockRefs);
        let target = namespace
            .ol_target_view(target_state)
            .expect("L1BlockRefs is OL-owned");
        add_missing_target_namespace(plan, namespace, target);
    }

    for (account_id, account_state) in target_state.iter_account_states() {
        let Ok(snark_account) = account_state.as_snark_account() else {
            continue;
        };

        let target = get_mmr_state_view(snark_account.inbox_mmr());
        if target.leaf_count == 0 {
            continue;
        }

        let namespace = MmrNamespace::new(MmrId::SnarkMsgInbox(account_id));
        if found_namespaces.contains(namespace.as_mmr_id()) {
            continue;
        }

        add_missing_target_namespace(plan, namespace, target);
    }
}

fn add_missing_target_namespace(
    plan: &mut MmrIndexRevertPlan,
    namespace: MmrNamespace,
    target_mmr_state: MmrStateView,
) {
    plan.inspected += 1;
    add_mmr_target_to_plan(
        plan,
        MmrNamespaceRecord {
            raw_mmr_id: namespace.raw_mmr_id(),
            namespace,
            leaf_count: 0,
        },
        target_mmr_state,
    );
}

/// Adds one namespace comparison to a revert plan.
///
/// A persisted MMR ahead of the target becomes a revert, an MMR equal to
/// the target is omitted, and an MMR behind the target becomes a preflight
/// error recorded in the plan.
fn add_mmr_target_to_plan(
    plan: &mut MmrIndexRevertPlan,
    record: MmrNamespaceRecord,
    target_mmr_state: MmrStateView,
) {
    let mmr_id = record.namespace.as_mmr_id().clone();
    let target_leaf_count = target_mmr_state.leaf_count;
    match record.leaf_count.cmp(&target_leaf_count) {
        Ordering::Greater => plan.revert_candidates.push(MmrIndexRevertCandidate {
            mmr_id,
            current_leaf_count: record.leaf_count,
            target_leaf_count,
            target_peaks: target_mmr_state.peaks,
        }),
        Ordering::Equal => {}
        Ordering::Less => plan.behind_target.push(MmrIndexBehindTarget {
            mmr_id,
            current_leaf_count: record.leaf_count,
            target_leaf_count,
        }),
    }
}

/// Returns the target inbox MMR state for a snark account.
///
/// Missing accounts and non-snark accounts have no target inbox MMR, so they
/// map to an empty MMR state. This lets revert remove MMRs created only by
/// blocks above the target.
fn get_target_snark_inbox_mmr_state(
    target_state: &OLState,
    account_id: &AccountId,
) -> MmrStateView {
    let Some(snark_account) = target_state
        .get_account_state(account_id)
        .and_then(|account_state| account_state.as_snark_account().ok())
    else {
        return MmrStateView {
            leaf_count: 0,
            peaks: Vec::new(),
        };
    };

    get_mmr_state_view(snark_account.inbox_mmr())
}

/// Converts a compact in-state MMR into the storage manager's state view.
///
/// Compact roots are stored lowest-height first, while [`MmrStateView`] uses
/// the same left-to-right peak order returned by
/// [`strata_storage::MmrIndexHandle::get_state_at`].
fn get_mmr_state_view(mmr: &Mmr64) -> MmrStateView {
    MmrStateView {
        leaf_count: mmr.num_entries(),
        peaks: mmr
            .roots
            .iter()
            .rev()
            .map(|peak| Hash::from(peak.0))
            .collect(),
    }
}

/// Validates that a revert plan can be executed without growing MMRs.
///
/// The function rejects MMRs that are behind the target and rejects removal
/// of the `L1BlockRefs` genesis sentinel. It performs no writes.
pub(crate) fn validate_mmr_index_revert_plan(
    plan: &MmrIndexRevertPlan,
) -> Result<(), DisplayedError> {
    if let Some(candidate) = plan.revert_candidates.iter().find(|candidate| {
        candidate.mmr_id == MmrId::L1BlockRefs && candidate.target_leaf_count == 0
    }) {
        return Err(DisplayedError::InternalError(
            "MMR l1-block-refs target would remove the genesis sentinel".to_string(),
            Box::new(candidate.clone()),
        ));
    }

    if let Some(behind_target) = plan.behind_target.first() {
        return Err(DisplayedError::InternalError(
            format!(
                "MMR {} is behind target",
                display_mmr_id(&behind_target.mmr_id)
            ),
            Box::new(behind_target.clone()),
        ));
    }

    Ok(())
}

/// Validates that every planned revert matches the target prefix in storage.
///
/// This check must run before destructive pops. It catches MMRs that are ahead
/// of the target count but do not have the target state as a prefix.
pub(crate) fn validate_mmr_index_revert_prefixes(
    db: &impl DatabaseBackend,
    plan: &MmrIndexRevertPlan,
) -> Result<(), DisplayedError> {
    with_mmr_index_manager(db, |mmr_index_manager| {
        validate_mmr_index_revert_prefixes_with_manager(mmr_index_manager, plan)
    })
}

fn validate_mmr_index_revert_prefixes_with_manager(
    mmr_index_manager: &MmrIndexManager,
    plan: &MmrIndexRevertPlan,
) -> Result<(), DisplayedError> {
    for candidate in &plan.revert_candidates {
        let handle = mmr_index_manager.get_handle(candidate.mmr_id.clone());
        let target_state = handle
            .get_state_at(candidate.target_leaf_count)
            .internal_error("Failed to read target MMR state before revert")?;
        if target_state.peaks != candidate.target_peaks {
            return Err(DisplayedError::InternalError(
                format!(
                    "MMR {} does not match target prefix at leaf count {}",
                    display_mmr_id(&candidate.mmr_id),
                    candidate.target_leaf_count
                ),
                Box::new(display_mmr_id(&candidate.mmr_id)),
            ));
        }
    }

    Ok(())
}

/// Executes a validated revert plan against the MMR index database.
///
/// Each leaf is removed through
/// [`strata_storage::MmrIndexHandle::pop_leaf_blocking`] via the storage manager,
/// preserving the canonical pop behavior. After each MMR is popped, the
/// function verifies the final leaf count and peaks against the target state
/// recorded in the plan.
pub(crate) fn execute_mmr_index_revert_plan(
    db: &impl DatabaseBackend,
    plan: &MmrIndexRevertPlan,
) -> Result<(), DisplayedError> {
    with_mmr_index_manager(db, |mmr_index_manager| {
        validate_mmr_index_revert_prefixes_with_manager(mmr_index_manager, plan)?;

        for candidate in &plan.revert_candidates {
            let handle = mmr_index_manager.get_handle(candidate.mmr_id.clone());
            for _ in 0..candidate.pop_count() {
                handle
                    .pop_leaf_blocking()
                    .internal_error("Failed to pop MMR leaf")?
                    .ok_or_else(|| {
                        DisplayedError::InternalError(
                            format!(
                                "MMR {} became empty before reaching target",
                                display_mmr_id(&candidate.mmr_id)
                            ),
                            Box::new(candidate.clone()),
                        )
                    })?;
            }

            let final_leaf_count = handle
                .get_num_leaves_blocking()
                .internal_error("Failed to read final MMR leaf count")?;
            if final_leaf_count != candidate.target_leaf_count {
                return Err(DisplayedError::InternalError(
                    format!(
                        "MMR {} final leaf count does not match target",
                        display_mmr_id(&candidate.mmr_id)
                    ),
                    Box::new(MmrIndexFinalLeafCountMismatch {
                        revert_candidate: candidate.clone(),
                        final_leaf_count,
                    }),
                ));
            }

            let final_state = handle
                .get_state_at(final_leaf_count)
                .internal_error("Failed to read final MMR state")?;
            if final_state.peaks != candidate.target_peaks {
                return Err(DisplayedError::InternalError(
                    format!(
                        "MMR {} final peaks do not match target state",
                        display_mmr_id(&candidate.mmr_id)
                    ),
                    Box::new(MmrIndexFinalPeaksMismatch {
                        revert_candidate: candidate.clone(),
                        final_peaks: final_state.peaks,
                    }),
                ));
            }
        }

        Ok(())
    })
}

fn with_mmr_index_manager<T>(
    db: &impl DatabaseBackend,
    f: impl FnOnce(&MmrIndexManager) -> Result<T, DisplayedError>,
) -> Result<T, DisplayedError> {
    let runtime = Runtime::new().internal_error("Failed to create MMR index runtime")?;
    let mmr_index_manager = MmrIndexManager::new(runtime.handle().clone(), db.mmr_index_db());
    f(&mmr_index_manager)
}

/// Prints the dry-run or execution summary for an MMR revert plan.
pub(crate) fn print_mmr_index_revert_summary(plan: &MmrIndexRevertPlan) {
    println!("MMRs to inspect: {}", plan.inspected);
    println!("MMRs skipped: {} ASM-owned", plan.asm_owned_skipped);
    println!("MMRs to revert: {}", plan.mmrs_to_revert());
    println!("MMR leaves to pop: {}", plan.leaves_to_pop());
    for candidate in &plan.revert_candidates {
        println!(
            "MMR revert: {} {} -> {} (pop {})",
            display_mmr_id(&candidate.mmr_id),
            candidate.current_leaf_count,
            candidate.target_leaf_count,
            candidate.pop_count()
        );
    }
    for behind_target in &plan.behind_target {
        println!(
            "MMR behind target: {} {} < {}",
            display_mmr_id(&behind_target.mmr_id),
            behind_target.current_leaf_count,
            behind_target.target_leaf_count
        );
    }
}

/// Converts namespace records into sorted summary output.
///
/// Empty namespaces are omitted from user output, and an owner filter narrows
/// the result after namespace decoding.
fn build_mmr_summary_info(
    records: Vec<MmrNamespaceRecord>,
    owner: Option<MmrOwner>,
) -> MmrSummaryInfo {
    let mut entries = records
        .into_iter()
        .filter(|record| record.leaf_count > 0)
        .map(build_mmr_summary_entry)
        .filter(|entry| owner.is_none_or(|owner| entry.owner == owner))
        .collect::<Vec<_>>();

    entries.sort_by(|a, b| {
        a.mmr_id
            .cmp(&b.mmr_id)
            .then_with(|| a.account.cmp(&b.account))
            .then_with(|| a.raw_mmr_id.cmp(&b.raw_mmr_id))
    });

    MmrSummaryInfo::new(entries)
}

/// Converts fetched leaf data into user-facing leaf output.
///
/// Known OL leaf types decode their preimages when possible and report whether
/// the decoded preimage hashes back to the stored leaf hash. Sentinel and ASM
/// leaves are shown without typed preimage decoding.
fn build_mmr_leaf_info(
    namespace: &MmrNamespace,
    leaf_data: MmrLeafData,
) -> Result<MmrLeafInfo, DisplayedError> {
    let MmrLeafData {
        raw_mmr_id,
        leaf_index,
        leaf_count,
        leaf_hash,
        preimage,
    } = leaf_data;
    let sentinel_dummy_leaf = namespace.is_sentinel_dummy_leaf(&leaf_hash);
    let decoded_preimage = if sentinel_dummy_leaf || !namespace.expects_typed_preimage() {
        None
    } else {
        let preimage = preimage.as_deref().ok_or_else(|| {
            DisplayedError::InternalError(
                format!(
                    "MMR leaf preimage is missing for {} at index {leaf_index}",
                    namespace.display_id()
                ),
                Box::new(leaf_index),
            )
        })?;

        namespace.decode_preimage(leaf_index, preimage).ok()
    };
    let (preimage_matches_hash, preimage_decoded) =
        decoded_preimage.map_or((None, None), |preimage_decoded| {
            (
                Some(preimage_decoded.expected_leaf_hash == leaf_hash),
                Some(preimage_decoded.preimage_decoded),
            )
        });
    let preimage_hex = if sentinel_dummy_leaf || !namespace.expects_typed_preimage() {
        None
    } else {
        preimage.map(hex::encode)
    };

    Ok(MmrLeafInfo {
        mmr_id: namespace.display_id(),
        owner: namespace.owner(),
        account: namespace.account(),
        leaf_index,
        leaf_count,
        leaf_hash: hash_to_hex(&leaf_hash),
        sentinel_dummy_leaf,
        preimage_hex,
        preimage_matches_hash,
        preimage_decoded,
        raw_mmr_id: hex::encode(raw_mmr_id),
    })
}

/// Converts one namespace record into a summary entry.
fn build_mmr_summary_entry(record: MmrNamespaceRecord) -> MmrSummaryEntry {
    let MmrNamespaceRecord {
        raw_mmr_id,
        namespace,
        leaf_count,
    } = record;
    let mmr_size = num_leaves_to_mmr_size(leaf_count);
    let raw_mmr_id = hex::encode(raw_mmr_id);

    MmrSummaryEntry {
        mmr_id: namespace.display_id(),
        owner: namespace.owner(),
        account: namespace.account(),
        leaf_count,
        mmr_size,
        raw_mmr_id,
    }
}

fn hash_to_hex(hash: &Hash) -> String {
    hex::encode(hash.as_bytes())
}

#[cfg(test)]
mod tests {
    use ssz::Encode;
    use strata_acct_types::{append_l1_block_rec_to_mmr, BitcoinAmount, L1BlockRecord, MsgPayload};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::MmrBatchWrite;
    use strata_identifiers::AccountId;
    use strata_ledger_types::{IAccountStateMut, ISnarkAccountStateMut};
    use strata_ol_params::{GenesisSnarkAccountData, OLParams};
    use strata_ol_state_types::{OLAccountState, WriteBatch};
    use strata_predicate::PredicateKey;
    use strata_storage::{MmrIndexHandle, MmrIndexManager};
    use tokio::runtime::Runtime;

    use super::*;

    fn genesis_target_state() -> OLState {
        OLState::from_genesis_params(&OLParams::default()).expect("valid genesis params")
    }

    fn l1_block_record(seed: u8) -> L1BlockRecord {
        L1BlockRecord::new([seed; 32], [seed.wrapping_add(0x80); 32])
    }

    fn target_state_with_l1_records(records: &[L1BlockRecord]) -> OLState {
        let mut state = genesis_target_state();
        let mut l1_block_refs_mmr = state.epoch_state().l1_block_refs_mmr().clone();
        for record in records {
            append_l1_block_rec_to_mmr(&mut l1_block_refs_mmr, record);
        }

        let mut batch = WriteBatch::<OLAccountState>::default();
        batch.epochal_writes_mut().l1_block_refs_mmr = Some(l1_block_refs_mmr);
        state
            .apply_write_batch(batch)
            .expect("apply target L1 refs MMR");
        state
    }

    fn snark_inbox_message(seed: u8) -> MessageEntry {
        let payload =
            MsgPayload::from_bytes(BitcoinAmount::from_sat(1), vec![seed]).expect("payload");
        MessageEntry::new(AccountId::new([seed; 32]), 0, payload)
    }

    fn target_state_with_snark_inbox(
        account_id: AccountId,
        messages: Vec<MessageEntry>,
    ) -> OLState {
        let mut params = OLParams::default();
        params.accounts.insert(
            account_id,
            GenesisSnarkAccountData {
                predicate: PredicateKey::always_accept(),
                inner_state: Hash::zero(),
                balance: BitcoinAmount::ZERO,
            },
        );

        let mut state = OLState::from_genesis_params(&params).expect("valid genesis params");
        let mut account = state
            .get_account_state(&account_id)
            .expect("genesis snark account")
            .clone();
        let snark_account = account
            .as_snark_account_mut()
            .expect("genesis account should be snark");
        for message in messages {
            snark_account
                .insert_inbox_message(message)
                .expect("insert inbox message");
        }

        let mut batch = WriteBatch::<OLAccountState>::default();
        batch.ledger_mut().update_account(account_id, account);
        state
            .apply_write_batch(batch)
            .expect("apply target snark inbox MMR");
        state
    }

    fn seed_l1_block_refs_index(handle: &MmrIndexHandle, records: &[L1BlockRecord]) {
        handle
            .append_leaf_blocking(MMR_SENTINEL_DUMMY_LEAF_HASH)
            .expect("append L1 sentinel");
        for record in records {
            handle
                .append_leaf_blocking(Hash::from(record.leaf_hash()))
                .expect("append L1 block ref");
        }
    }

    #[test]
    fn gets_mmr_namespace_records_from_backend() {
        let db = get_test_sled_backend();
        let l1_block_refs = MmrId::L1BlockRefs.to_bytes();
        let empty_asm = MmrId::Asm.to_bytes();

        let mut batch = MmrBatchWrite::default();
        batch.entry(l1_block_refs.clone()).set_leaf_count(2);
        batch.entry(empty_asm.clone()).set_leaf_count(0);
        db.mmr_index_db()
            .apply_update(batch)
            .expect("seed MMR leaf counts");

        let mut records = get_mmr_namespace_records(db.as_ref()).expect("get MMR records");
        records.sort_by(|a, b| a.raw_mmr_id.cmp(&b.raw_mmr_id));

        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|record| {
            record.raw_mmr_id == l1_block_refs
                && record.namespace.as_mmr_id() == &MmrId::L1BlockRefs
                && record.leaf_count == 2
        }));
        assert!(records.iter().any(|record| {
            record.raw_mmr_id == empty_asm
                && record.namespace.as_mmr_id() == &MmrId::Asm
                && record.leaf_count == 0
        }));
    }

    #[test]
    fn get_mmr_namespace_records_rejects_invalid_namespace_id() {
        let db = get_test_sled_backend();
        let invalid_mmr_id = vec![0xff];

        let mut batch = MmrBatchWrite::default();
        batch.entry(invalid_mmr_id).set_leaf_count(4);
        db.mmr_index_db()
            .apply_update(batch)
            .expect("seed invalid MMR leaf count");

        let err =
            get_mmr_namespace_records(db.as_ref()).expect_err("invalid namespace should fail");

        assert!(err
            .to_string()
            .contains("MMR namespace id ff is not a known MmrId"));
    }

    #[test]
    fn builds_mmr_index_revert_plan_from_target_state() {
        let target_state = genesis_target_state();
        let account_id = AccountId::new([0x22; 32]);

        let plan = build_mmr_index_revert_plan(
            &target_state,
            vec![
                MmrNamespaceRecord {
                    raw_mmr_id: MmrId::Asm.to_bytes(),
                    namespace: MmrNamespace::new(MmrId::Asm),
                    leaf_count: 3,
                },
                MmrNamespaceRecord {
                    raw_mmr_id: MmrId::L1BlockRefs.to_bytes(),
                    namespace: MmrNamespace::new(MmrId::L1BlockRefs),
                    leaf_count: 3,
                },
                MmrNamespaceRecord {
                    raw_mmr_id: MmrId::SnarkMsgInbox(account_id).to_bytes(),
                    namespace: MmrNamespace::new(MmrId::SnarkMsgInbox(account_id)),
                    leaf_count: 2,
                },
            ],
        );

        assert_eq!(plan.inspected, 3);
        assert_eq!(plan.asm_owned_skipped, 1);
        assert_eq!(plan.revert_candidates.len(), 2);
        assert_eq!(plan.behind_target, vec![]);
        assert_eq!(plan.mmrs_to_revert(), 2);
        assert_eq!(plan.leaves_to_pop(), 4);

        let l1_candidate = plan
            .revert_candidates
            .iter()
            .find(|candidate| candidate.mmr_id == MmrId::L1BlockRefs)
            .expect("L1 revert");
        assert_eq!(l1_candidate.current_leaf_count, 3);
        assert_eq!(l1_candidate.target_leaf_count, 1);
        assert_eq!(
            l1_candidate.target_peaks,
            get_mmr_state_view(target_state.epoch_state().l1_block_refs_mmr()).peaks
        );

        let snark_candidate = plan
            .revert_candidates
            .iter()
            .find(|candidate| matches!(candidate.mmr_id, MmrId::SnarkMsgInbox(_)))
            .expect("snark revert");
        assert_eq!(snark_candidate.current_leaf_count, 2);
        assert_eq!(snark_candidate.target_leaf_count, 0);
        assert_eq!(snark_candidate.target_peaks, Vec::<Hash>::new());
    }

    #[test]
    fn validates_mmr_index_revert_plan_rejects_behind_target() {
        let target_state = genesis_target_state();
        let plan = build_mmr_index_revert_plan(
            &target_state,
            vec![MmrNamespaceRecord {
                raw_mmr_id: MmrId::L1BlockRefs.to_bytes(),
                namespace: MmrNamespace::new(MmrId::L1BlockRefs),
                leaf_count: 0,
            }],
        );

        assert_eq!(
            plan.behind_target,
            vec![MmrIndexBehindTarget {
                mmr_id: MmrId::L1BlockRefs,
                current_leaf_count: 0,
                target_leaf_count: 1,
            }]
        );
        let err = validate_mmr_index_revert_plan(&plan).expect_err("behind target should fail");

        assert!(err
            .to_string()
            .contains("MMR l1-block-refs is behind target"));
    }

    #[test]
    fn build_mmr_index_revert_plan_requires_l1_block_refs_when_missing() {
        let target_state = genesis_target_state();
        let plan = build_mmr_index_revert_plan(&target_state, Vec::new());

        assert_eq!(plan.inspected, 1);
        assert_eq!(
            plan.behind_target,
            vec![MmrIndexBehindTarget {
                mmr_id: MmrId::L1BlockRefs,
                current_leaf_count: 0,
                target_leaf_count: 1,
            }]
        );

        let err = validate_mmr_index_revert_plan(&plan)
            .expect_err("missing L1BlockRefs should be behind target");
        assert!(err
            .to_string()
            .contains("MMR l1-block-refs is behind target"));
    }

    #[test]
    fn build_mmr_index_revert_plan_requires_target_snark_inbox_when_missing() {
        let account_id = AccountId::new([0x44; 32]);
        let target_state =
            target_state_with_snark_inbox(account_id, vec![snark_inbox_message(0x55)]);
        let plan = build_mmr_index_revert_plan(
            &target_state,
            vec![MmrNamespaceRecord {
                raw_mmr_id: MmrId::L1BlockRefs.to_bytes(),
                namespace: MmrNamespace::new(MmrId::L1BlockRefs),
                leaf_count: 1,
            }],
        );

        assert_eq!(plan.inspected, 2);
        assert_eq!(
            plan.behind_target,
            vec![MmrIndexBehindTarget {
                mmr_id: MmrId::SnarkMsgInbox(account_id),
                current_leaf_count: 0,
                target_leaf_count: 1,
            }]
        );

        let err = validate_mmr_index_revert_plan(&plan)
            .expect_err("missing target snark inbox should be behind target");
        assert!(err.to_string().contains(&format!(
            "MMR snark-msg-inbox:{account_id} is behind target"
        )));
    }

    #[test]
    fn validates_mmr_index_revert_plan_rejects_l1_sentinel_removal() {
        let plan = MmrIndexRevertPlan {
            inspected: 1,
            asm_owned_skipped: 0,
            revert_candidates: vec![MmrIndexRevertCandidate {
                mmr_id: MmrId::L1BlockRefs,
                current_leaf_count: 1,
                target_leaf_count: 0,
                target_peaks: Vec::new(),
            }],
            behind_target: Vec::new(),
        };

        let err = validate_mmr_index_revert_plan(&plan).expect_err("sentinel removal should fail");

        assert!(err
            .to_string()
            .contains("MMR l1-block-refs target would remove the genesis sentinel"));
    }

    #[test]
    fn get_mmr_index_revert_plan_rejects_invalid_namespace_id() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();
        let mut batch = MmrBatchWrite::default();
        batch.entry(vec![0xff]).set_leaf_count(2);
        db.mmr_index_db()
            .apply_update(batch)
            .expect("seed invalid namespace");

        let err = get_mmr_index_revert_plan(db.as_ref(), &target_state)
            .expect_err("invalid namespace should fail");

        assert!(err
            .to_string()
            .contains("MMR namespace id ff is not a known MmrId"));
    }

    #[test]
    fn execute_mmr_index_revert_plan_pops_to_target_count_and_peaks() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();
        let account_id = AccountId::new([0x77; 32]);
        let runtime = Runtime::new().expect("create runtime");
        let manager = MmrIndexManager::new(runtime.handle().clone(), db.mmr_index_db());
        let l1_handle = manager.get_handle(MmrId::L1BlockRefs);
        l1_handle
            .append_leaf_blocking(MMR_SENTINEL_DUMMY_LEAF_HASH)
            .expect("append L1 sentinel");
        l1_handle
            .append_leaf_blocking(Hash::from([0x11; 32]))
            .expect("append extra L1 leaf");
        l1_handle
            .append_leaf_blocking(Hash::from([0x22; 32]))
            .expect("append extra L1 leaf");
        let snark_handle = manager.get_handle(MmrId::SnarkMsgInbox(account_id));
        snark_handle
            .append_leaf_blocking(Hash::from([0x33; 32]))
            .expect("append snark leaf");

        let plan =
            get_mmr_index_revert_plan(db.as_ref(), &target_state).expect("build revert plan");
        assert_eq!(plan.mmrs_to_revert(), 2);
        assert_eq!(plan.leaves_to_pop(), 3);

        execute_mmr_index_revert_plan(db.as_ref(), &plan).expect("execute revert");

        assert_eq!(
            l1_handle.get_num_leaves_blocking().expect("L1 leaf count"),
            1
        );
        assert_eq!(
            l1_handle.get_state_at(1).expect("L1 state").peaks,
            get_mmr_state_view(target_state.epoch_state().l1_block_refs_mmr()).peaks
        );
        assert_eq!(
            snark_handle
                .get_num_leaves_blocking()
                .expect("snark leaf count"),
            0
        );
        assert_eq!(
            db.mmr_index_db()
                .get_leaf_count(MmrId::SnarkMsgInbox(account_id).to_bytes())
                .expect("snark leaf count row"),
            0
        );
    }

    #[test]
    fn execute_mmr_index_revert_plan_rejects_non_prefix_target_before_pop() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();
        let runtime = Runtime::new().expect("create runtime");
        let manager = MmrIndexManager::new(runtime.handle().clone(), db.mmr_index_db());
        let l1_handle = manager.get_handle(MmrId::L1BlockRefs);
        l1_handle
            .append_leaf_blocking(Hash::from([0x11; 32]))
            .expect("append non-target L1 first leaf");
        l1_handle
            .append_leaf_blocking(Hash::from([0x22; 32]))
            .expect("append extra L1 leaf");

        let plan =
            get_mmr_index_revert_plan(db.as_ref(), &target_state).expect("build revert plan");
        assert_eq!(plan.mmrs_to_revert(), 1);
        assert_eq!(plan.revert_candidates[0].current_leaf_count, 2);
        assert_eq!(plan.revert_candidates[0].target_leaf_count, 1);

        let err =
            execute_mmr_index_revert_plan(db.as_ref(), &plan).expect_err("non-prefix should fail");

        assert!(err.to_string().contains("does not match target prefix"));
        assert_eq!(
            l1_handle.get_num_leaves_blocking().expect("L1 leaf count"),
            2
        );
    }

    #[test]
    fn execute_mmr_index_revert_plan_verifies_multi_peak_l1_target() {
        let db = get_test_sled_backend();
        let target_records = [l1_block_record(1), l1_block_record(2)];
        let target_state = target_state_with_l1_records(&target_records);
        let runtime = Runtime::new().expect("create runtime");
        let manager = MmrIndexManager::new(runtime.handle().clone(), db.mmr_index_db());
        let l1_handle = manager.get_handle(MmrId::L1BlockRefs);
        seed_l1_block_refs_index(&l1_handle, &target_records);
        l1_handle
            .append_leaf_blocking(Hash::from([0x33; 32]))
            .expect("append extra L1 leaf");
        l1_handle
            .append_leaf_blocking(Hash::from([0x44; 32]))
            .expect("append extra L1 leaf");

        let plan =
            get_mmr_index_revert_plan(db.as_ref(), &target_state).expect("build revert plan");
        assert_eq!(plan.mmrs_to_revert(), 1);
        assert_eq!(plan.leaves_to_pop(), 2);
        assert_eq!(plan.revert_candidates[0].target_leaf_count, 3);
        assert_eq!(plan.revert_candidates[0].target_peaks.len(), 2);

        execute_mmr_index_revert_plan(db.as_ref(), &plan).expect("execute revert");

        assert_eq!(
            l1_handle.get_num_leaves_blocking().expect("L1 leaf count"),
            3
        );
        let final_peaks = l1_handle.get_state_at(3).expect("L1 state").peaks;
        assert_eq!(final_peaks.len(), 2);
        assert_eq!(
            final_peaks,
            get_mmr_state_view(target_state.epoch_state().l1_block_refs_mmr()).peaks
        );
    }

    #[test]
    fn builds_mmr_summary_and_skips_empty_namespaces() {
        let account_id = AccountId::new([0x11; 32]);
        let records = vec![
            MmrNamespaceRecord {
                raw_mmr_id: MmrId::L1BlockRefs.to_bytes(),
                namespace: MmrNamespace::new(MmrId::L1BlockRefs),
                leaf_count: 2,
            },
            MmrNamespaceRecord {
                raw_mmr_id: MmrId::Asm.to_bytes(),
                namespace: MmrNamespace::new(MmrId::Asm),
                leaf_count: 0,
            },
            MmrNamespaceRecord {
                raw_mmr_id: MmrId::SnarkMsgInbox(account_id).to_bytes(),
                namespace: MmrNamespace::new(MmrId::SnarkMsgInbox(account_id)),
                leaf_count: 1,
            },
        ];

        let summary = build_mmr_summary_info(records, None);

        assert_eq!(summary.mmr_count, 2);
        assert_eq!(summary.entries.len(), 2);
        assert_eq!(summary.entries[0].mmr_id, "l1-block-refs");
        assert_eq!(summary.entries[0].leaf_count, 2);
        assert_eq!(
            summary.entries[1].mmr_id,
            format!("snark-msg-inbox:{account_id}")
        );
        assert_eq!(summary.entries[1].account, Some(account_id.to_string()));
        assert_eq!(summary.entries[1].leaf_count, 1);
        assert!(summary.entries.iter().all(|entry| entry.leaf_count > 0));
    }

    #[test]
    fn builds_mmr_summary_filters_by_owner() {
        let account_id = AccountId::new([0x33; 32]);
        let records = vec![
            MmrNamespaceRecord {
                raw_mmr_id: MmrId::Asm.to_bytes(),
                namespace: MmrNamespace::new(MmrId::Asm),
                leaf_count: 3,
            },
            MmrNamespaceRecord {
                raw_mmr_id: MmrId::L1BlockRefs.to_bytes(),
                namespace: MmrNamespace::new(MmrId::L1BlockRefs),
                leaf_count: 2,
            },
            MmrNamespaceRecord {
                raw_mmr_id: MmrId::SnarkMsgInbox(account_id).to_bytes(),
                namespace: MmrNamespace::new(MmrId::SnarkMsgInbox(account_id)),
                leaf_count: 1,
            },
        ];

        let all_summary = build_mmr_summary_info(records.clone(), None);
        let asm_summary = build_mmr_summary_info(records.clone(), Some(MmrOwner::Asm));
        let ol_summary = build_mmr_summary_info(records, Some(MmrOwner::OL));

        assert_eq!(all_summary.mmr_count, 3);

        assert_eq!(asm_summary.mmr_count, 1);
        assert_eq!(asm_summary.entries[0].mmr_id, "asm");
        assert!(asm_summary
            .entries
            .iter()
            .all(|entry| entry.owner == MmrOwner::Asm));

        assert_eq!(ol_summary.mmr_count, 2);
        assert_eq!(ol_summary.entries[0].mmr_id, "l1-block-refs");
        assert!(ol_summary.entries[1].mmr_id.starts_with("snark-msg-inbox:"));
        assert!(ol_summary
            .entries
            .iter()
            .all(|entry| entry.owner == MmrOwner::OL));
    }

    #[test]
    fn rejects_invalid_owner_filter() {
        let err = "bad-owner"
            .parse::<MmrOwner>()
            .expect_err("invalid owner should fail");

        assert_eq!(err.to_string(), "must be 'ol' or 'asm'");
    }

    #[test]
    fn parses_user_facing_mmr_ids() {
        let account_id = AccountId::new([0x44; 32]);

        assert_eq!(
            "asm"
                .parse::<MmrIdInput>()
                .expect("parse asm")
                .namespace()
                .as_mmr_id(),
            &MmrId::Asm
        );
        assert_eq!(
            "l1-block-refs"
                .parse::<MmrIdInput>()
                .expect("parse l1 block refs")
                .namespace()
                .as_mmr_id(),
            &MmrId::L1BlockRefs
        );
        assert_eq!(
            "l1_block_refs"
                .parse::<MmrIdInput>()
                .expect("parse normalized l1 block refs")
                .namespace()
                .as_mmr_id(),
            &MmrId::L1BlockRefs
        );
        assert_eq!(
            format!("snark-msg-inbox:{account_id}")
                .parse::<MmrIdInput>()
                .expect("parse snark inbox")
                .namespace()
                .as_mmr_id(),
            &MmrId::SnarkMsgInbox(account_id)
        );
        assert_eq!(
            format!("snark_msg_inbox:{account_id}")
                .parse::<MmrIdInput>()
                .expect("parse normalized snark inbox")
                .namespace()
                .as_mmr_id(),
            &MmrId::SnarkMsgInbox(account_id)
        );
    }

    #[test]
    fn rejects_invalid_mmr_id() {
        let err = "l1-blocks".parse::<MmrIdInput>().expect_err("invalid id");

        assert_eq!(
            err.to_string(),
            "must be 'asm', 'l1-block-refs', or 'snark-msg-inbox:<account-hex>'"
        );
    }

    #[test]
    fn rejects_invalid_snark_inbox_account_id() {
        let err = "snark-msg-inbox:abcd"
            .parse::<MmrIdInput>()
            .expect_err("short account id should fail");

        assert_eq!(
            err.to_string(),
            "account id must be exactly 32 bytes (got 2 bytes)"
        );
    }

    #[test]
    fn reads_l1_block_ref_leaf_from_backend() {
        let db = get_test_sled_backend();
        let mmr_id = MmrId::L1BlockRefs;
        let raw_mmr_id = mmr_id.to_bytes();
        let leaf_index = 7;
        let leaf_pos = LeafPos::new(leaf_index);
        let record = L1BlockRecord::new([0x11; 32], [0x22; 32]);
        let leaf_hash = Hash::from(record.leaf_hash());
        let preimage = record.as_ssz_bytes();

        let mut batch = MmrBatchWrite::default();
        let entry = batch.entry(raw_mmr_id.clone());
        entry.set_leaf_count(8);
        entry.put_node(leaf_pos.to_node_pos(), leaf_hash);
        entry.put_preimage(leaf_pos, preimage.clone());
        db.mmr_index_db()
            .apply_update(batch)
            .expect("seed MMR leaf");

        let namespace = MmrNamespace::new(mmr_id);
        let leaf_data =
            get_mmr_leaf_data(db.as_ref(), &namespace, leaf_index).expect("get MMR leaf data");
        let leaf = build_mmr_leaf_info(&namespace, leaf_data).expect("build MMR leaf info");

        assert_eq!(leaf.mmr_id, "l1-block-refs");
        assert_eq!(leaf.owner, MmrOwner::OL);
        assert_eq!(leaf.account, None);
        assert_eq!(leaf.leaf_index, leaf_index);
        assert_eq!(leaf.leaf_count, 8);
        assert_eq!(leaf.leaf_hash, hash_to_hex(&leaf_hash));
        assert!(!leaf.sentinel_dummy_leaf);
        assert_eq!(leaf.preimage_matches_hash, Some(true));
        assert_eq!(
            leaf.preimage_decoded,
            Some(MmrPreimageDecoded::L1BlockRef {
                height: leaf_index,
                block_hash: hex::encode([0x11; 32]),
                wtxids_root: hex::encode([0x22; 32]),
            })
        );
        assert_eq!(leaf.preimage_hex, Some(hex::encode(preimage)));
        assert_eq!(leaf.raw_mmr_id, hex::encode(raw_mmr_id));
    }

    #[test]
    fn reads_snark_inbox_leaf_from_backend() {
        let db = get_test_sled_backend();
        let account_id = AccountId::new([0x99; 32]);
        let source = AccountId::new([0x44; 32]);
        let mmr_id = MmrId::SnarkMsgInbox(account_id);
        let raw_mmr_id = mmr_id.to_bytes();
        let leaf_pos = LeafPos::new(0);
        let payload =
            MsgPayload::from_bytes(BitcoinAmount::from_sat(42), vec![0xaa, 0xbb]).expect("payload");
        let message = MessageEntry::new(source, 12, payload);
        let leaf_hash = message.compute_msg_commitment();
        let preimage = message.as_ssz_bytes();

        let mut batch = MmrBatchWrite::default();
        let entry = batch.entry(raw_mmr_id.clone());
        entry.set_leaf_count(1);
        entry.put_node(leaf_pos.to_node_pos(), leaf_hash);
        entry.put_preimage(leaf_pos, preimage.clone());
        db.mmr_index_db()
            .apply_update(batch)
            .expect("seed MMR leaf");

        let namespace = MmrNamespace::new(mmr_id);
        let leaf_data = get_mmr_leaf_data(db.as_ref(), &namespace, 0).expect("get MMR leaf data");
        let leaf = build_mmr_leaf_info(&namespace, leaf_data).expect("build MMR leaf info");

        assert_eq!(leaf.mmr_id, format!("snark-msg-inbox:{account_id}"));
        assert_eq!(leaf.owner, MmrOwner::OL);
        assert_eq!(leaf.account, Some(account_id.to_string()));
        assert_eq!(leaf.leaf_hash, hash_to_hex(&leaf_hash));
        assert!(!leaf.sentinel_dummy_leaf);
        assert_eq!(leaf.preimage_matches_hash, Some(true));
        assert_eq!(
            leaf.preimage_decoded,
            Some(MmrPreimageDecoded::SnarkMsgInbox {
                source: source.to_string(),
                inclusion_epoch: 12,
                payload_len: 2,
                payload_hash: hash_to_hex(&hash::raw(&[0xaa, 0xbb])),
            })
        );
        assert_eq!(leaf.preimage_hex, Some(hex::encode(preimage)));
        assert_eq!(leaf.raw_mmr_id, hex::encode(raw_mmr_id));
    }

    #[test]
    fn builds_mmr_leaf_info_reports_hash_mismatch() {
        let mmr_id = MmrId::L1BlockRefs;
        let record = L1BlockRecord::new([0x11; 32], [0x22; 32]);
        let leaf_data = MmrLeafData {
            raw_mmr_id: mmr_id.to_bytes(),
            leaf_index: 0,
            leaf_count: 1,
            leaf_hash: Hash::from([0x55; 32]),
            preimage: Some(record.as_ssz_bytes()),
        };

        let namespace = MmrNamespace::new(mmr_id);
        let leaf = build_mmr_leaf_info(&namespace, leaf_data).expect("build MMR leaf info");

        assert_eq!(leaf.preimage_matches_hash, Some(false));
        assert!(!leaf.sentinel_dummy_leaf);
        assert!(matches!(
            leaf.preimage_decoded,
            Some(MmrPreimageDecoded::L1BlockRef { .. })
        ));
        assert!(leaf.preimage_hex.is_some());
    }

    #[test]
    fn build_mmr_leaf_info_omits_decoded_fields_for_invalid_typed_preimage() {
        let mmr_id = MmrId::L1BlockRefs;
        let leaf_data = MmrLeafData {
            raw_mmr_id: mmr_id.to_bytes(),
            leaf_index: 0,
            leaf_count: 1,
            leaf_hash: Hash::from([0x55; 32]),
            preimage: Some(vec![0xaa, 0xbb, 0xcc]),
        };

        let namespace = MmrNamespace::new(mmr_id);
        let leaf =
            build_mmr_leaf_info(&namespace, leaf_data).expect("invalid preimage should print");

        assert_eq!(leaf.preimage_matches_hash, None);
        assert_eq!(leaf.preimage_decoded, None);
        assert_eq!(leaf.preimage_hex, Some("aabbcc".to_string()));
    }

    #[test]
    fn get_mmr_leaf_data_rejects_out_of_range_index() {
        let db = get_test_sled_backend();
        let mmr_id = MmrId::L1BlockRefs;
        let mut batch = MmrBatchWrite::default();
        batch.entry(mmr_id.to_bytes()).set_leaf_count(1);
        db.mmr_index_db()
            .apply_update(batch)
            .expect("seed MMR leaf count");

        let namespace = MmrNamespace::new(mmr_id);
        let err =
            get_mmr_leaf_data(db.as_ref(), &namespace, 1).expect_err("index should be rejected");

        assert!(err
            .to_string()
            .contains("MMR leaf index 1 is out of range for leaf count 1"));
    }

    #[test]
    fn get_mmr_leaf_data_reports_empty_or_missing_mmr() {
        let db = get_test_sled_backend();

        let namespace = MmrNamespace::new(MmrId::L1BlockRefs);
        let err = get_mmr_leaf_data(db.as_ref(), &namespace, 0)
            .expect_err("empty or missing MMR should be rejected");

        assert!(err
            .to_string()
            .contains("MMR l1-block-refs is not found or empty"));
    }

    #[test]
    fn build_mmr_leaf_info_errors_on_missing_typed_preimage() {
        let mmr_id = MmrId::L1BlockRefs;
        let leaf_hash = Hash::from([0x66; 32]);
        let leaf_data = MmrLeafData {
            raw_mmr_id: mmr_id.to_bytes(),
            leaf_index: 0,
            leaf_count: 1,
            leaf_hash,
            preimage: None,
        };

        let namespace = MmrNamespace::new(mmr_id);
        let err =
            build_mmr_leaf_info(&namespace, leaf_data).expect_err("missing preimage should fail");

        assert!(err
            .to_string()
            .contains("MMR leaf preimage is missing for l1-block-refs at index 0"));
    }

    #[test]
    fn build_mmr_leaf_info_allows_hash_only_asm_leaf() {
        let mmr_id = MmrId::Asm;
        let leaf_hash = Hash::from([0x66; 32]);
        let leaf_data = MmrLeafData {
            raw_mmr_id: mmr_id.to_bytes(),
            leaf_index: 0,
            leaf_count: 1,
            leaf_hash,
            preimage: None,
        };

        let namespace = MmrNamespace::new(mmr_id);
        let leaf =
            build_mmr_leaf_info(&namespace, leaf_data).expect("ASM leaf should be hash-only");

        assert_eq!(leaf.leaf_hash, hex::encode([0x66; 32]));
        assert!(!leaf.sentinel_dummy_leaf);
        assert_eq!(leaf.preimage_matches_hash, None);
        assert_eq!(leaf.preimage_decoded, None);
        assert_eq!(leaf.preimage_hex, None);
    }

    #[test]
    fn build_mmr_leaf_info_marks_sentinel_dummy_leaf() {
        let mmr_id = MmrId::L1BlockRefs;
        let leaf_data = MmrLeafData {
            raw_mmr_id: mmr_id.to_bytes(),
            leaf_index: 0,
            leaf_count: 1,
            leaf_hash: MMR_SENTINEL_DUMMY_LEAF_HASH,
            preimage: None,
        };

        let namespace = MmrNamespace::new(mmr_id);
        let leaf = build_mmr_leaf_info(&namespace, leaf_data).expect("build MMR leaf info");

        assert_eq!(leaf.leaf_hash, hash_to_hex(&MMR_SENTINEL_DUMMY_LEAF_HASH));
        assert!(leaf.sentinel_dummy_leaf);
        assert_eq!(leaf.preimage_matches_hash, None);
        assert_eq!(leaf.preimage_decoded, None);
        assert_eq!(leaf.preimage_hex, None);
    }
}
