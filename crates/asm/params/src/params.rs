use serde::{Deserialize, Serialize, de::Error};
use strata_btc_types::GenesisL1View;
use strata_l1_txfmt::MagicBytes;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsmParams {
    #[serde(with = "serde_magic_bytes")]
    magic: MagicBytes,

    l1_view: GenesisL1View,
}

/// Serialize/deserialize [`MagicBytes`] using its Display/FromStr implementation.
mod serde_magic_bytes {
    use std::str::FromStr;

    use serde::{Deserializer, Serializer};

    use super::*;

    pub(super) fn serialize<S: Serializer>(v: &MagicBytes, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<MagicBytes, D::Error> {
        let s = String::deserialize(d)?;
        MagicBytes::from_str(&s).map_err(D::Error::custom)
    }
}
