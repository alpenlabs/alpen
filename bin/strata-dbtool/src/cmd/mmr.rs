use std::{
    fmt::{self, Display},
    str::FromStr,
};

use argh::FromArgs;
use ssz::Decode;
use strata_acct_types::{L1BlockRecord, MessageEntry};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_crypto::hash;
use strata_db_types::{
    backend::DatabaseBackend, mmr_index::MmrIndexDatabase, num_leaves_to_mmr_size, LeafPos, MmrId,
    RawMmrId,
};
use strata_identifiers::{AccountId, Hash};
use strata_ol_state_types::MMR_SENTINEL_DUMMY_LEAF_HASH;

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

#[derive(Clone, Debug)]
struct MmrNamespaceRecord {
    raw_mmr_id: RawMmrId,
    namespace: MmrNamespace,
    leaf_count: u64,
}

#[derive(Debug)]
struct MmrLeafData {
    raw_mmr_id: RawMmrId,
    leaf_index: u64,
    leaf_count: u64,
    leaf_hash: Hash,
    preimage: Option<Vec<u8>>,
}

#[derive(Debug)]
struct DecodedPreimage {
    expected_leaf_hash: Hash,
    preimage_decoded: MmrPreimageDecoded,
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
    use strata_acct_types::{BitcoinAmount, MsgPayload};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::MmrBatchWrite;
    use strata_identifiers::AccountId;

    use super::*;

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
