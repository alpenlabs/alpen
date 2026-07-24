use std::{
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
use strata_ol_mmr_index::{
    build_mmr_index_reconcile_plan, MmrIndexEntry, MmrIndexReconcilePlan, MmrIndexTruncation,
    OLMmrIndexError,
};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{OLState, MMR_SENTINEL_DUMMY_LEAF_HASH};
use strata_storage::MmrIndexManager;
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
                Box::new(()),
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
        self.mmr_id.fmt(f)
    }
}

/// Normalizes CLI input to the canonical dash-separated MMR id form.
///
/// The command prints dash ids for copy-paste use, but accepts snake_case input
/// because those names mirror Rust enum variants and are easy to type.
fn normalize_mmr_id_part(input: &str) -> String {
    input.replace('_', "-")
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

/// Describes a post-truncate leaf-count mismatch for one MMR.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MmrIndexFinalLeafCountMismatch {
    truncation: MmrIndexTruncation,
    final_leaf_count: u64,
}

/// Describes a post-truncate state mismatch for one MMR: the leaf count matched
/// the target but the native MMR state did not.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MmrIndexFinalStateMismatch {
    truncation: MmrIndexTruncation,
    final_state: Mmr64,
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

    let leaf_count = mmr_db
        .get_leaf_count(raw_mmr_id.clone())
        .internal_error("Failed to read MMR leaf count")?;
    if leaf_count == 0 {
        return Err(DisplayedError::UserError(
            format!("MMR {namespace} is not found or empty"),
            Box::new(()),
        ));
    }
    if leaf_index >= leaf_count {
        return Err(DisplayedError::UserError(
            format!("MMR leaf index {leaf_index} is out of range for leaf count {leaf_count}"),
            Box::new(()),
        ));
    }

    let leaf_hash = mmr_db
        .get_node(raw_mmr_id.clone(), leaf_pos.to_node_pos())
        .internal_error("Failed to read MMR leaf hash")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                format!("MMR leaf hash is missing for {namespace} at index {leaf_index}"),
                Box::new(()),
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
/// kept by `target_state`, rejecting indexes that are behind or divergent.
/// It performs only reads; mutation is a separate step.
pub(crate) fn build_mmr_index_revert_plan(
    db: &impl DatabaseBackend,
    target_state: &OLState,
) -> Result<MmrIndexReconcilePlan, DisplayedError> {
    with_mmr_index_manager(db, |mmr_index_manager| {
        let target_state_accessor = MemoryStateBaseLayer::new(target_state.clone());
        let records = get_mmr_index_entries(db, mmr_index_manager)?;
        build_mmr_index_reconcile_plan(
            &target_state_accessor,
            records,
            target_state.iter_snark_account_ids(),
        )
        .map_err(|err| {
            if matches!(&err, OLMmrIndexError::BehindTarget { .. }) {
                DisplayedError::UserError(
                    "MMR index is behind target OL state".to_string(),
                    Box::new(err),
                )
            } else {
                DisplayedError::InternalError(
                    "Failed to build MMR index revert plan".to_string(),
                    Box::new(err),
                )
            }
        })
    })
}

/// Reads every persisted MMR namespace and its state for revert planning.
fn get_mmr_index_entries(
    db: &impl DatabaseBackend,
    mmr_index_manager: &MmrIndexManager,
) -> Result<Vec<MmrIndexEntry>, DisplayedError> {
    let records = get_mmr_namespace_records(db)?;
    let mut entries = Vec::with_capacity(records.len());

    for record in records {
        let mmr_id = record.namespace.as_mmr_id().clone();
        let state = mmr_index_manager
            .get_handle(mmr_id.clone())
            .get_state_at_blocking(record.leaf_count)
            .internal_error(format!("Failed to read MMR state for {}", record.namespace))?;
        entries.push(MmrIndexEntry::new(mmr_id, state));
    }

    Ok(entries)
}

/// Validates that every planned revert matches the target prefix in storage.
///
/// This check must run before destructive pops. It catches MMRs that are ahead
/// of the target count but do not have the target state as a prefix.
pub(crate) fn validate_mmr_index_revert_prefixes(
    db: &impl DatabaseBackend,
    plan: &MmrIndexReconcilePlan,
) -> Result<(), DisplayedError> {
    with_mmr_index_manager(db, |mmr_index_manager| {
        validate_mmr_index_revert_prefixes_inner(mmr_index_manager, plan)
    })
}

fn validate_mmr_index_revert_prefixes_inner(
    mmr_index_manager: &MmrIndexManager,
    plan: &MmrIndexReconcilePlan,
) -> Result<(), DisplayedError> {
    for truncation in plan.truncations() {
        let target_mmr_state = truncation.target();
        let handle = mmr_index_manager.get_handle(truncation.mmr_id().clone());
        let indexed_mmr_state = handle
            .get_state_at_blocking(target_mmr_state.num_entries())
            .internal_error("Failed to read target MMR state before revert")?;
        if &indexed_mmr_state != target_mmr_state {
            return Err(DisplayedError::InternalError(
                format!(
                    "MMR {} does not match target prefix at leaf count {}",
                    truncation.mmr_id(),
                    target_mmr_state.num_entries()
                ),
                Box::new(()),
            ));
        }
    }

    Ok(())
}

/// Executes a validated revert plan against the MMR index database.
///
/// Each MMR is truncated once through the storage manager. After each truncate,
/// the function rechecks the final leaf count and native MMR state as a
/// defense-in-depth guard against storage regressions.
pub(crate) fn execute_mmr_index_revert_plan(
    db: &impl DatabaseBackend,
    plan: &MmrIndexReconcilePlan,
) -> Result<(), DisplayedError> {
    with_mmr_index_manager(db, |mmr_index_manager| {
        validate_mmr_index_revert_prefixes_inner(mmr_index_manager, plan)?;

        for truncation in plan.truncations() {
            let target_mmr_state = truncation.target();
            let handle = mmr_index_manager.get_handle(truncation.mmr_id().clone());
            handle
                .truncate_to_leaf_count_blocking(target_mmr_state.num_entries())
                .internal_error(format!("Failed to truncate MMR {}", truncation.mmr_id()))?;

            let final_leaf_count = handle
                .get_leaf_count_blocking()
                .internal_error("Failed to read final MMR leaf count")?;
            if final_leaf_count != target_mmr_state.num_entries() {
                return Err(DisplayedError::InternalError(
                    format!(
                        "MMR {} final leaf count does not match target",
                        truncation.mmr_id()
                    ),
                    Box::new(MmrIndexFinalLeafCountMismatch {
                        truncation: truncation.clone(),
                        final_leaf_count,
                    }),
                ));
            }

            let indexed_mmr_state = handle
                .get_state_at_blocking(final_leaf_count)
                .internal_error("Failed to read final MMR state")?;
            if &indexed_mmr_state != target_mmr_state {
                return Err(DisplayedError::InternalError(
                    format!(
                        "MMR {} final state does not match target",
                        truncation.mmr_id()
                    ),
                    Box::new(MmrIndexFinalStateMismatch {
                        truncation: truncation.clone(),
                        final_state: indexed_mmr_state,
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
pub(crate) fn print_mmr_index_revert_summary(plan: &MmrIndexReconcilePlan) {
    println!("MMRs to inspect: {}", plan.inspected());
    println!("MMRs skipped: {} ASM-owned", plan.asm_owned_skipped());
    println!("MMRs to revert: {}", plan.truncation_count());
    println!("MMR leaves to remove: {}", plan.leaves_to_remove_count());
    for truncation in plan.truncations() {
        let target_mmr_state = truncation.target();
        println!(
            "MMR revert: {} {} -> {} (remove {})",
            truncation.mmr_id(),
            truncation.index_leaf_count(),
            target_mmr_state.num_entries(),
            truncation.leaves_to_remove()
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
                format!("MMR leaf preimage is missing for {namespace} at index {leaf_index}"),
                Box::new(()),
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
        mmr_id: namespace.to_string(),
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
        mmr_id: namespace.to_string(),
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
    use strata_merkle::MmrState;
    use strata_ol_params::OLParams;
    use strata_ol_state_types::{OLAccountState, WriteBatch};
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

    fn persisted_mmr_index_entry(manager: &MmrIndexManager, mmr_id: MmrId) -> MmrIndexEntry {
        let handle = manager.get_handle(mmr_id.clone());
        let leaf_count = handle.get_leaf_count_blocking().expect("read leaf count");
        let state = handle
            .get_state_at_blocking(leaf_count)
            .expect("read MMR state");

        MmrIndexEntry::new(mmr_id, state)
    }

    #[test]
    fn test_namespace_listing_includes_leaf_counts() {
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
    fn test_namespace_listing_rejects_invalid_namespace_id() {
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
    fn test_revert_plan_skips_asm() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();
        let target_state_accessor = MemoryStateBaseLayer::new(target_state.clone());
        let account_id = AccountId::new([0x22; 32]);
        let runtime = Runtime::new().expect("create runtime");
        let manager = MmrIndexManager::new(runtime.handle().clone(), db.mmr_index_db());
        let asm_handle = manager.get_handle(MmrId::Asm);
        for leaf in [0x11, 0x22, 0x33] {
            asm_handle
                .append_leaf_blocking(Hash::from([leaf; 32]))
                .expect("append ASM leaf");
        }
        let l1_records = [l1_block_record(1), l1_block_record(2)];
        seed_l1_block_refs_index(&manager.get_handle(MmrId::L1BlockRefs), &l1_records);
        let snark_handle = manager.get_handle(MmrId::SnarkMsgInbox(account_id));
        for leaf in [0x44, 0x55] {
            snark_handle
                .append_leaf_blocking(Hash::from([leaf; 32]))
                .expect("append snark leaf");
        }

        let plan = build_mmr_index_reconcile_plan(
            &target_state_accessor,
            vec![
                persisted_mmr_index_entry(&manager, MmrId::Asm),
                persisted_mmr_index_entry(&manager, MmrId::L1BlockRefs),
                persisted_mmr_index_entry(&manager, MmrId::SnarkMsgInbox(account_id)),
            ],
            target_state.iter_snark_account_ids(),
        )
        .expect("valid plan");

        assert_eq!(plan.inspected(), 3);
        assert_eq!(plan.asm_owned_skipped(), 1);
        assert_eq!(plan.truncation_count(), 2);
        assert_eq!(plan.leaves_to_remove_count(), 4);

        let truncate_ids = plan
            .truncations()
            .iter()
            .map(|truncation| truncation.mmr_id().clone())
            .collect::<Vec<_>>();
        assert!(truncate_ids.contains(&MmrId::L1BlockRefs));
        assert!(truncate_ids.contains(&MmrId::SnarkMsgInbox(account_id)));
    }

    #[test]
    fn test_revert_plan_rejects_index_behind_target() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();

        let err = build_mmr_index_revert_plan(db.as_ref(), &target_state)
            .expect_err("behind target should fail");

        let DisplayedError::UserError(message, _) = err else {
            panic!("expected user error for behind target");
        };
        assert_eq!(message, "MMR index is behind target OL state");
    }

    #[test]
    fn test_revert_plan_rejects_invalid_namespace_id() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();
        let mut batch = MmrBatchWrite::default();
        batch.entry(vec![0xff]).set_leaf_count(2);
        db.mmr_index_db()
            .apply_update(batch)
            .expect("seed invalid namespace");

        let err = build_mmr_index_revert_plan(db.as_ref(), &target_state)
            .expect_err("invalid namespace should fail");

        assert!(err
            .to_string()
            .contains("MMR namespace id ff is not a known MmrId"));
    }

    #[test]
    fn test_revert_plan_rejects_same_count_state_mismatch() {
        let db = get_test_sled_backend();
        let target_state = genesis_target_state();
        let runtime = Runtime::new().expect("create runtime");
        let manager = MmrIndexManager::new(runtime.handle().clone(), db.mmr_index_db());
        let l1_handle = manager.get_handle(MmrId::L1BlockRefs);
        l1_handle
            .append_leaf_blocking(Hash::from([0x11; 32]))
            .expect("append non-target L1 sentinel slot");

        let err = build_mmr_index_revert_plan(db.as_ref(), &target_state)
            .expect_err("same-count state mismatch should fail");

        let DisplayedError::InternalError(message, _) = err else {
            panic!("expected internal error for state mismatch");
        };
        assert_eq!(message, "Failed to build MMR index revert plan");
    }

    #[test]
    fn test_revert_execution_truncates_indexes_to_target() {
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
            build_mmr_index_revert_plan(db.as_ref(), &target_state).expect("build revert plan");
        assert_eq!(plan.truncation_count(), 2);
        assert_eq!(plan.leaves_to_remove_count(), 3);

        execute_mmr_index_revert_plan(db.as_ref(), &plan).expect("execute revert");

        assert_eq!(
            l1_handle.get_leaf_count_blocking().expect("L1 leaf count"),
            1
        );
        assert_eq!(
            &l1_handle.get_state_at_blocking(1).expect("L1 state"),
            target_state.epoch_state().l1_block_refs_mmr()
        );
        assert_eq!(
            snark_handle
                .get_leaf_count_blocking()
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
    fn test_revert_execution_rejects_non_prefix_target() {
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
            build_mmr_index_revert_plan(db.as_ref(), &target_state).expect("build revert plan");
        assert_eq!(plan.truncation_count(), 1);
        let truncation = plan.truncations().first().expect("index to truncate");
        assert_eq!(truncation.index_leaf_count(), 2);
        assert_eq!(truncation.target().num_entries(), 1);

        let err =
            execute_mmr_index_revert_plan(db.as_ref(), &plan).expect_err("non-prefix should fail");

        assert!(err.to_string().contains("does not match target prefix"));
        assert_eq!(
            l1_handle.get_leaf_count_blocking().expect("L1 leaf count"),
            2
        );
    }

    #[test]
    fn test_revert_execution_preserves_multi_peak_target() {
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
            build_mmr_index_revert_plan(db.as_ref(), &target_state).expect("build revert plan");
        assert_eq!(plan.truncation_count(), 1);
        assert_eq!(plan.leaves_to_remove_count(), 2);
        let truncation = plan.truncations().first().expect("index to truncate");
        let target_mmr_state = truncation.target();
        assert_eq!(target_mmr_state.num_entries(), 3);
        assert_eq!(target_mmr_state.iter_peaks().count(), 2);

        execute_mmr_index_revert_plan(db.as_ref(), &plan).expect("execute revert");

        assert_eq!(
            l1_handle.get_leaf_count_blocking().expect("L1 leaf count"),
            3
        );
        let final_state = l1_handle.get_state_at_blocking(3).expect("L1 state");
        assert_eq!(final_state.iter_peaks().count(), 2);
        assert_eq!(&final_state, target_state.epoch_state().l1_block_refs_mmr());
    }

    #[test]
    fn test_summary_skips_empty_namespaces() {
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
    fn test_summary_filters_by_owner() {
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
    fn test_invalid_owner_filter_is_rejected() {
        let err = "bad-owner"
            .parse::<MmrOwner>()
            .expect_err("invalid owner should fail");

        assert_eq!(err.to_string(), "must be 'ol' or 'asm'");
    }

    #[test]
    fn test_user_facing_mmr_ids_are_parsed() {
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
    fn test_mmr_ids_round_trip_through_cli_input() {
        let account_id = AccountId::new([0x55; 32]);

        for mmr_id in [
            MmrId::Asm,
            MmrId::L1BlockRefs,
            MmrId::SnarkMsgInbox(account_id),
        ] {
            let input = mmr_id.to_string();
            let namespace = MmrNamespace::from_cli_input(&input).expect("parse displayed MMR id");

            assert_eq!(namespace.as_mmr_id(), &mmr_id);
        }
    }

    #[test]
    fn test_invalid_mmr_id_is_rejected() {
        let err = "l1-blocks".parse::<MmrIdInput>().expect_err("invalid id");

        assert_eq!(
            err.to_string(),
            "must be 'asm', 'l1-block-refs', or 'snark-msg-inbox:<account-hex>'"
        );
    }

    #[test]
    fn test_invalid_snark_inbox_account_id_is_rejected() {
        let err = "snark-msg-inbox:abcd"
            .parse::<MmrIdInput>()
            .expect_err("short account id should fail");

        assert_eq!(
            err.to_string(),
            "account id must be exactly 32 bytes (got 2 bytes)"
        );
    }

    #[test]
    fn test_l1_block_ref_leaf_is_decoded() {
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
    fn test_snark_inbox_leaf_is_decoded() {
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
    fn test_leaf_info_reports_hash_mismatch() {
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
    fn test_invalid_typed_preimage_omits_decoded_fields() {
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
    fn test_out_of_range_leaf_index_is_rejected() {
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
    fn test_empty_or_missing_mmr_leaf_query_is_rejected() {
        let db = get_test_sled_backend();

        let namespace = MmrNamespace::new(MmrId::L1BlockRefs);
        let err = get_mmr_leaf_data(db.as_ref(), &namespace, 0)
            .expect_err("empty or missing MMR should be rejected");

        assert!(err
            .to_string()
            .contains("MMR l1-block-refs is not found or empty"));
    }

    #[test]
    fn test_missing_typed_preimage_is_rejected() {
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
    fn test_hash_only_asm_leaf_is_allowed() {
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
    fn test_sentinel_dummy_leaf_is_marked() {
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
