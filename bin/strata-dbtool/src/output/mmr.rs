//! MMR index formatting implementations.

use std::{
    fmt::{self, Display},
    str::FromStr,
};

use super::{
    helpers::{porcelain_bool, porcelain_field},
    traits::Formattable,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct UnsupportedMmrOwner;

impl Display for UnsupportedMmrOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "must be 'ol' or 'asm'")
    }
}

/// Subsystem that maintains an MMR namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MmrOwner {
    Asm,
    OL,
}

impl FromStr for MmrOwner {
    type Err = UnsupportedMmrOwner;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "asm" => Ok(MmrOwner::Asm),
            "ol" => Ok(MmrOwner::OL),
            _ => Err(UnsupportedMmrOwner),
        }
    }
}

impl Display for MmrOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Asm => "asm",
            Self::OL => "ol",
        })
    }
}

/// One indexed MMR namespace summary.
#[derive(Debug, serde::Serialize)]
pub(crate) struct MmrSummaryEntry {
    pub(crate) mmr_id: String,
    pub(crate) owner: MmrOwner,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account: Option<String>,
    pub(crate) leaf_count: u64,
    pub(crate) mmr_size: u64,
    pub(crate) raw_mmr_id: String,
}

/// Summary of indexed MMR namespaces.
#[derive(Debug, serde::Serialize)]
pub(crate) struct MmrSummaryInfo {
    pub(crate) mmr_count: u64,
    pub(crate) entries: Vec<MmrSummaryEntry>,
}

impl MmrSummaryInfo {
    pub(crate) fn new(entries: Vec<MmrSummaryEntry>) -> Self {
        let mmr_count = u64::try_from(entries.len()).expect("MMR count should fit in u64");
        Self { mmr_count, entries }
    }
}

impl Formattable for MmrSummaryInfo {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();
        output.push(porcelain_field("mmr_count", self.mmr_count));

        for (index, entry) in self.entries.iter().enumerate() {
            output.push(porcelain_field(
                &format!("mmr.{index}.mmr_id"),
                &entry.mmr_id,
            ));
            output.push(porcelain_field(&format!("mmr.{index}.owner"), entry.owner));
            if let Some(account) = &entry.account {
                output.push(porcelain_field(&format!("mmr.{index}.account"), account));
            }
            output.push(porcelain_field(
                &format!("mmr.{index}.leaf_count"),
                entry.leaf_count,
            ));
        }

        output.join("\n")
    }
}

/// Decoded typed preimage for one MMR leaf.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MmrPreimageDecoded {
    L1BlockRef {
        height: u64,
        block_hash: String,
        wtxids_root: String,
    },
    SnarkMsgInbox {
        source: String,
        inclusion_epoch: u32,
        payload_len: u64,
        payload_hash: String,
    },
}

/// One MMR leaf lookup result.
#[derive(Debug, serde::Serialize)]
pub(crate) struct MmrLeafInfo {
    pub(crate) mmr_id: String,
    pub(crate) owner: MmrOwner,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account: Option<String>,
    pub(crate) leaf_index: u64,
    pub(crate) leaf_count: u64,
    pub(crate) leaf_hash: String,
    #[serde(skip_serializing_if = "is_false")]
    pub(crate) sentinel_dummy_leaf: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) preimage_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) preimage_matches_hash: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) preimage_decoded: Option<MmrPreimageDecoded>,
    pub(crate) raw_mmr_id: String,
}

impl Formattable for MmrLeafInfo {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();
        output.push(porcelain_field("mmr_id", &self.mmr_id));
        output.push(porcelain_field("owner", self.owner));
        if let Some(account) = &self.account {
            output.push(porcelain_field("account", account));
        }
        output.push(porcelain_field("leaf_index", self.leaf_index));
        output.push(porcelain_field("leaf_count", self.leaf_count));
        output.push(porcelain_field("leaf_hash", &self.leaf_hash));
        if self.sentinel_dummy_leaf {
            output.push(porcelain_field("sentinel_dummy_leaf", "true"));
        }
        if let Some(preimage_hex) = &self.preimage_hex {
            output.push(porcelain_field("preimage_hex", preimage_hex));
        }
        if let Some(preimage_matches_hash) = self.preimage_matches_hash {
            output.push(porcelain_field(
                "preimage_matches_hash",
                porcelain_bool(preimage_matches_hash),
            ));
        }
        if let Some(preimage_decoded) = &self.preimage_decoded {
            match preimage_decoded {
                MmrPreimageDecoded::L1BlockRef {
                    height,
                    block_hash,
                    wtxids_root,
                } => {
                    output.push(porcelain_field("preimage_decoded.kind", "l1_block_ref"));
                    output.push(porcelain_field("preimage_decoded.height", height));
                    output.push(porcelain_field("preimage_decoded.block_hash", block_hash));
                    output.push(porcelain_field("preimage_decoded.wtxids_root", wtxids_root));
                }
                MmrPreimageDecoded::SnarkMsgInbox {
                    source,
                    inclusion_epoch,
                    payload_len,
                    payload_hash,
                } => {
                    output.push(porcelain_field("preimage_decoded.kind", "snark_msg_inbox"));
                    output.push(porcelain_field("preimage_decoded.source", source));
                    output.push(porcelain_field(
                        "preimage_decoded.inclusion_epoch",
                        inclusion_epoch,
                    ));
                    output.push(porcelain_field("preimage_decoded.payload_len", payload_len));
                    output.push(porcelain_field(
                        "preimage_decoded.payload_hash",
                        payload_hash,
                    ));
                }
            }
        }

        output.join("\n")
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmr_summary_porcelain_lists_entries() {
        let info = MmrSummaryInfo::new(vec![
            MmrSummaryEntry {
                mmr_id: "l1-block-refs".to_string(),
                owner: MmrOwner::OL,
                account: None,
                leaf_count: 3,
                mmr_size: 4,
                raw_mmr_id: "02".to_string(),
            },
            MmrSummaryEntry {
                mmr_id:
                    "snark-msg-inbox:1111111111111111111111111111111111111111111111111111111111111111"
                        .to_string(),
                owner: MmrOwner::OL,
                account: Some(
                    "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
                ),
                leaf_count: 5,
                mmr_size: 8,
                raw_mmr_id: "0102".to_string(),
            },
        ]);

        let output = info.format_porcelain();

        assert_eq!(
            output,
            "mmr_count: 2\n\
             mmr.0.mmr_id: l1-block-refs\n\
             mmr.0.owner: ol\n\
             mmr.0.leaf_count: 3\n\
             mmr.1.mmr_id: snark-msg-inbox:1111111111111111111111111111111111111111111111111111111111111111\n\
             mmr.1.owner: ol\n\
             mmr.1.account: 1111111111111111111111111111111111111111111111111111111111111111\n\
             mmr.1.leaf_count: 5"
        );
    }

    #[test]
    fn mmr_summary_json_includes_rich_fields() {
        let info = MmrSummaryInfo::new(vec![MmrSummaryEntry {
            mmr_id: "l1-block-refs".to_string(),
            owner: MmrOwner::OL,
            account: None,
            leaf_count: 3,
            mmr_size: 4,
            raw_mmr_id: "02".to_string(),
        }]);

        let value = serde_json::to_value(&info).expect("serialize summary");

        assert_eq!(value["mmr_count"], 1);
        assert_eq!(value["entries"][0]["mmr_id"], "l1-block-refs");
        assert_eq!(value["entries"][0]["owner"], "ol");
        assert!(value["entries"][0].get("account").is_none());
        assert_eq!(value["entries"][0]["leaf_count"], 3);
        assert_eq!(value["entries"][0]["mmr_size"], 4);
        assert_eq!(value["entries"][0]["raw_mmr_id"], "02");
    }

    #[test]
    fn mmr_leaf_porcelain_includes_leaf_details() {
        let info = MmrLeafInfo {
            mmr_id: "l1-block-refs".to_string(),
            owner: MmrOwner::OL,
            account: None,
            leaf_index: 7,
            leaf_count: 8,
            leaf_hash: "1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
            sentinel_dummy_leaf: false,
            preimage_hex: Some("aabbcc".to_string()),
            preimage_matches_hash: Some(true),
            preimage_decoded: Some(MmrPreimageDecoded::L1BlockRef {
                height: 7,
                block_hash: "2222222222222222222222222222222222222222222222222222222222222222"
                    .to_string(),
                wtxids_root: "3333333333333333333333333333333333333333333333333333333333333333"
                    .to_string(),
            }),
            raw_mmr_id: "02".to_string(),
        };

        let output = info.format_porcelain();

        assert_eq!(
            output,
            "mmr_id: l1-block-refs\n\
             owner: ol\n\
             leaf_index: 7\n\
             leaf_count: 8\n\
             leaf_hash: 1111111111111111111111111111111111111111111111111111111111111111\n\
             preimage_hex: aabbcc\n\
             preimage_matches_hash: true\n\
             preimage_decoded.kind: l1_block_ref\n\
             preimage_decoded.height: 7\n\
             preimage_decoded.block_hash: 2222222222222222222222222222222222222222222222222222222222222222\n\
             preimage_decoded.wtxids_root: 3333333333333333333333333333333333333333333333333333333333333333"
        );
    }

    #[test]
    fn mmr_leaf_json_includes_rich_fields() {
        let info = MmrLeafInfo {
            mmr_id:
                "snark-msg-inbox:2222222222222222222222222222222222222222222222222222222222222222"
                    .to_string(),
            owner: MmrOwner::OL,
            account: Some(
                "2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            ),
            leaf_index: 1,
            leaf_count: 2,
            leaf_hash: "3333333333333333333333333333333333333333333333333333333333333333"
                .to_string(),
            sentinel_dummy_leaf: false,
            preimage_matches_hash: None,
            preimage_decoded: None,
            preimage_hex: None,
            raw_mmr_id: "0102".to_string(),
        };

        let value = serde_json::to_value(&info).expect("serialize leaf");

        assert_eq!(
            value["mmr_id"],
            "snark-msg-inbox:2222222222222222222222222222222222222222222222222222222222222222"
        );
        assert_eq!(value["owner"], "ol");
        assert_eq!(
            value["account"],
            "2222222222222222222222222222222222222222222222222222222222222222"
        );
        assert_eq!(value["leaf_index"], 1);
        assert_eq!(value["leaf_count"], 2);
        assert_eq!(
            value["leaf_hash"],
            "3333333333333333333333333333333333333333333333333333333333333333"
        );
        assert!(value.get("sentinel_dummy_leaf").is_none());
        assert!(value.get("preimage_matches_hash").is_none());
        assert!(value.get("preimage_decoded").is_none());
        assert!(value.get("preimage_hex").is_none());
        assert_eq!(value["raw_mmr_id"], "0102");
    }

    #[test]
    fn mmr_leaf_outputs_sentinel_without_preimage_fields() {
        let info = MmrLeafInfo {
            mmr_id: "l1-block-refs".to_string(),
            owner: MmrOwner::OL,
            account: None,
            leaf_index: 0,
            leaf_count: 1,
            leaf_hash: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                .to_string(),
            sentinel_dummy_leaf: true,
            preimage_matches_hash: None,
            preimage_decoded: None,
            preimage_hex: None,
            raw_mmr_id: "02".to_string(),
        };

        let output = info.format_porcelain();

        assert_eq!(
            output,
            "mmr_id: l1-block-refs\n\
             owner: ol\n\
             leaf_index: 0\n\
             leaf_count: 1\n\
             leaf_hash: ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n\
             sentinel_dummy_leaf: true"
        );

        let value = serde_json::to_value(&info).expect("serialize leaf");

        assert_eq!(
            value["leaf_hash"],
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        );
        assert_eq!(value["sentinel_dummy_leaf"], true);
        assert!(value.get("preimage_matches_hash").is_none());
        assert!(value.get("preimage_decoded").is_none());
        assert!(value.get("preimage_hex").is_none());
    }
}
