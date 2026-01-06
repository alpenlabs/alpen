use std::{
    fmt::{self, Debug, Display},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use strata_identifiers::Slot;
use strata_primitives::{Buf32, OLBlockId};

/// Identifies a block by tag, hash, or slot number.
#[derive(Clone)]
pub enum OLBlockOrTag {
    /// The most recent block produced.
    Latest,
    /// The most recent block confirmed on L1.
    Confirmed,
    /// The most recent block finalized on L1.
    Finalized,
    /// A specific block by its hash.
    OLBlockId(OLBlockId),
    /// A specific block by its slot number.
    Slot(Slot),
}

impl From<OLBlockId> for OLBlockOrTag {
    fn from(value: OLBlockId) -> Self {
        Self::OLBlockId(value)
    }
}

impl From<Slot> for OLBlockOrTag {
    fn from(value: Slot) -> Self {
        Self::Slot(value)
    }
}

impl Serialize for OLBlockOrTag {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            OLBlockOrTag::Latest => serializer.serialize_str("latest"),
            OLBlockOrTag::Confirmed => serializer.serialize_str("confirmed"),
            OLBlockOrTag::Finalized => serializer.serialize_str("finalized"),
            OLBlockOrTag::OLBlockId(olblock_id) => {
                serializer.serialize_str(&format!("0x{}", hex::encode(olblock_id.as_ref())))
            }
            OLBlockOrTag::Slot(slot) => serializer.serialize_str(&slot.to_string()),
        }
    }
}

#[allow(
    clippy::absolute_paths,
    clippy::allow_attributes,
    reason = "distinguish serde Error"
)]
impl<'de> Deserialize<'de> for OLBlockOrTag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for OLBlockOrTag {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "latest" => Self::Latest,
            "confirmed" => Self::Confirmed,
            "finalized" => Self::Finalized,
            s => {
                // check if it a blockhash
                if s.starts_with("0x") && s.len() == 66 {
                    let mut bytes = [0u8; 32];
                    hex::decode_to_slice(&s[2..], &mut bytes)
                        .map_err(|_| "invalid blockhash hex")?;

                    Self::OLBlockId(OLBlockId::from(Buf32::from(bytes)))
                } else {
                    // try to parse slot
                    let slot: u64 = s.parse().map_err(|_| "invalid slot")?;
                    Self::Slot(slot)
                }
            }
        })
    }
}

impl Display for OLBlockOrTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Slot(x) => write!(f, "{x}"),
            Self::OLBlockId(olblock_id) => write!(f, "0x{}", hex::encode(olblock_id.as_ref())),
            Self::Latest => f.pad("latest"),
            Self::Confirmed => f.pad("confirmed"),
            Self::Finalized => f.pad("finalized"),
        }
    }
}

impl Debug for OLBlockOrTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(block: &OLBlockOrTag) -> OLBlockOrTag {
        let serialized = serde_json::to_string(block).unwrap();
        serde_json::from_str(&serialized).unwrap()
    }

    #[test]
    fn test_latest_roundtrip() {
        let block = OLBlockOrTag::Latest;
        let result = roundtrip(&block);
        assert!(matches!(result, OLBlockOrTag::Latest));
    }

    #[test]
    fn test_confirmed_roundtrip() {
        let block = OLBlockOrTag::Confirmed;
        let result = roundtrip(&block);
        assert!(matches!(result, OLBlockOrTag::Confirmed));
    }

    #[test]
    fn test_finalized_roundtrip() {
        let block = OLBlockOrTag::Finalized;
        let result = roundtrip(&block);
        assert!(matches!(result, OLBlockOrTag::Finalized));
    }

    #[test]
    fn test_hash_roundtrip() {
        let bytes = [0xab; 32];
        let block = OLBlockOrTag::OLBlockId(OLBlockId::from(Buf32::from(bytes)));
        let result = roundtrip(&block);
        match result {
            OLBlockOrTag::OLBlockId(id) => assert_eq!(id.as_ref(), &bytes),
            _ => panic!("expected Hash variant"),
        }
    }

    #[test]
    fn test_slot_roundtrip() {
        let block = OLBlockOrTag::Slot(12345);
        let result = roundtrip(&block);
        match result {
            OLBlockOrTag::Slot(slot) => assert_eq!(slot, 12345),
            _ => panic!("expected Slot variant"),
        }
    }

    #[test]
    fn test_deserialize_invalid_hex() {
        let json = r#""0xZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ""#;
        let result: Result<OLBlockOrTag, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_wrong_length_hex() {
        // 0x + 64 chars is valid (32 bytes), test with wrong length
        let json = r#""0xabcd""#;
        let result: Result<OLBlockOrTag, _> = serde_json::from_str(json);
        // This should fail to parse as hash (wrong length) and fail as slot (not numeric)
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_invalid_slot() {
        let json = r#""not_a_number""#;
        let result: Result<OLBlockOrTag, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_case_insensitive_tags() {
        let cases = [
            "LATEST",
            "Latest",
            "CONFIRMED",
            "Confirmed",
            "FINALIZED",
            "Finalized",
        ];
        for case in cases {
            let json = format!(r#""{case}""#);
            let result: Result<OLBlockOrTag, _> = serde_json::from_str(&json);
            assert!(result.is_ok(), "failed to parse: {case}");
        }
    }
}
