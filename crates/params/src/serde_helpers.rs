//! Serde helper modules for serialization/deserialization of Bitcoin types.
use bitcoin::{Amount, absolute};
use serde::{Deserialize, Deserializer, Serializer, de::Error};
use strata_l1_txfmt::MagicBytes;

/// Serialize/deserialize [`Amount`] as integer satoshis ([`u64`]).
pub mod serde_amount_sat {
    use super::*;

    pub fn serialize<S: Serializer>(v: &Amount, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(v.to_sat())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Amount, D::Error> {
        let sats = u64::deserialize(d)?;
        Ok(Amount::from_sat(sats))
    }
}

/// Serialize/deserialize [`absolute::Height`] as [`u64`].
pub mod serde_height {
    use super::*;

    pub fn serialize<S: Serializer>(v: &absolute::Height, s: S) -> Result<S::Ok, S::Error> {
        let height_u64 = v.to_consensus_u32() as u64;
        s.serialize_u64(height_u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<absolute::Height, D::Error> {
        let height = u64::deserialize(d)?;
        absolute::Height::from_consensus(height as u32)
            .map_err(|e| D::Error::custom(format!("invalid block height {height}: {e}")))
    }
}

/// Serialize/deserialize [`MagicBytes`] using its Display/FromStr implementation.
pub mod serde_magic_bytes {
    use std::str::FromStr;

    use super::*;

    pub fn serialize<S: Serializer>(v: &MagicBytes, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<MagicBytes, D::Error> {
        let s = String::deserialize(d)?;
        MagicBytes::from_str(&s).map_err(D::Error::custom)
    }
}
