use std::{
    fmt::{self, Debug, Display},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
/// Identifies a CSS-safe OL state point by tag.
#[derive(Clone, Copy)]
pub enum OLBlockTag {
    /// The most recent block produced.
    Latest,
    /// The most recent block confirmed on L1.
    Confirmed,
    /// The most recent block finalized on L1.
    Finalized,
}

impl Serialize for OLBlockTag {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            OLBlockTag::Latest => serializer.serialize_str("latest"),
            OLBlockTag::Confirmed => serializer.serialize_str("confirmed"),
            OLBlockTag::Finalized => serializer.serialize_str("finalized"),
        }
    }
}

#[allow(
    clippy::absolute_paths,
    clippy::allow_attributes,
    reason = "distinguish serde Error"
)]
impl<'de> Deserialize<'de> for OLBlockTag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for OLBlockTag {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "latest" => Self::Latest,
            "confirmed" => Self::Confirmed,
            "finalized" => Self::Finalized,
            _ => return Err("invalid OL block tag"),
        })
    }
}

impl Display for OLBlockTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Latest => f.pad("latest"),
            Self::Confirmed => f.pad("confirmed"),
            Self::Finalized => f.pad("finalized"),
        }
    }
}

impl Debug for OLBlockTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(block: &OLBlockTag) -> OLBlockTag {
        let serialized = serde_json::to_string(block).unwrap();
        serde_json::from_str(&serialized).unwrap()
    }

    #[test]
    fn test_latest_roundtrip() {
        let block = OLBlockTag::Latest;
        let result = roundtrip(&block);
        assert!(matches!(result, OLBlockTag::Latest));
    }

    #[test]
    fn test_confirmed_roundtrip() {
        let block = OLBlockTag::Confirmed;
        let result = roundtrip(&block);
        assert!(matches!(result, OLBlockTag::Confirmed));
    }

    #[test]
    fn test_finalized_roundtrip() {
        let block = OLBlockTag::Finalized;
        let result = roundtrip(&block);
        assert!(matches!(result, OLBlockTag::Finalized));
    }

    #[test]
    fn test_deserialize_invalid_tag() {
        let json = r#""not_a_number""#;
        let result: Result<OLBlockTag, _> = serde_json::from_str(json);
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
            let result: Result<OLBlockTag, _> = serde_json::from_str(&json);
            assert!(result.is_ok(), "failed to parse: {case}");
        }
    }
}
