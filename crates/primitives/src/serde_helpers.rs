//! Serde helper modules for serialization/deserialization of Bitcoin types.
use bitcoin::{absolute, Amount};
use serde::{Deserialize, Deserializer, Serializer};

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
        use serde::de::Error;
        let height = u64::deserialize(d)?;
        absolute::Height::from_consensus(height as u32)
            .map_err(|e| D::Error::custom(format!("invalid block height {height}: {e}")))
    }
}

pub mod serde_hex_bytes {
    #[cfg(feature = "jsonschema")]
    use schemars::{
        r#gen::SchemaGenerator,
        schema::{InstanceType, Metadata, Schema, SchemaObject},
    };
    use serde::{Deserialize, Serialize};
    use strata_identifiers::L2BlockId;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct HexBytes(#[serde(with = "hex::serde")] pub Vec<u8>);

    #[cfg(feature = "jsonschema")]
    impl schemars::JsonSchema for HexBytes {
        fn schema_name() -> String {
            "HexBytes".to_owned()
        }

        fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
            SchemaObject {
                instance_type: Some(InstanceType::String.into()),
                format: Some("hex".to_owned()),
                metadata: Some(Box::new(Metadata {
                    description: Some("Hex-encoded byte array".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            }
            .into()
        }
    }

    impl HexBytes {
        pub fn into_inner(self) -> Vec<u8> {
            self.0
        }
    }

    impl From<Vec<u8>> for HexBytes {
        fn from(value: Vec<u8>) -> Self {
            HexBytes(value)
        }
    }

    impl From<&[u8]> for HexBytes {
        fn from(value: &[u8]) -> Self {
            HexBytes(value.to_vec())
        }
    }

    impl From<Box<[u8]>> for HexBytes {
        fn from(value: Box<[u8]>) -> Self {
            HexBytes(value.into_vec())
        }
    }

    impl From<HexBytes> for Vec<u8> {
        fn from(value: HexBytes) -> Self {
            value.0
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct HexBytes32(#[serde(with = "hex::serde")] pub [u8; 32]);

    // NOTE: keeping for backward compatibility
    impl From<&L2BlockId> for HexBytes32 {
        fn from(value: &L2BlockId) -> Self {
            Self(*value.as_ref())
        }
    }

    impl From<[u8; 32]> for HexBytes32 {
        fn from(value: [u8; 32]) -> Self {
            Self(value)
        }
    }

    impl From<HexBytes32> for [u8; 32] {
        fn from(value: HexBytes32) -> Self {
            value.0
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct HexBytes64(#[serde(with = "hex::serde")] pub [u8; 64]);

    impl From<[u8; 64]> for HexBytes64 {
        fn from(value: [u8; 64]) -> Self {
            Self(value)
        }
    }

    impl From<HexBytes64> for [u8; 64] {
        fn from(value: HexBytes64) -> Self {
            value.0
        }
    }
}
