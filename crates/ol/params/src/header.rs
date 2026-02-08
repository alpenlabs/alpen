//! Header parameters for the parent of the genesis block.

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, Epoch, OLBlockId};

/// Header parameters for the parent of the genesis block.
///
/// These describe the block immediately preceding genesis, not the genesis
/// block itself. All fields have sensible defaults. If not provided,
/// `timestamp`, `slot`, and `epoch` default to 0, while `parent_blkid`,
/// `body_root`, and `logs_root` default to their zero/null values.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct HeaderParams {
    /// Block timestamp. Defaults to 0.
    #[serde(default)]
    pub timestamp: u64,

    /// Slot. Defaults to 0.
    #[serde(default)]
    pub slot: u64,

    /// Epoch number. Defaults to 0.
    #[serde(default)]
    pub epoch: Epoch,

    /// Parent block ID. Defaults to `OLBlockId::null()`.
    #[serde(default = "OLBlockId::null")]
    pub parent_blkid: OLBlockId,

    /// Body root hash. Defaults to `Buf32::zero()`.
    #[serde(default = "Buf32::zero")]
    pub body_root: Buf32,

    /// Logs root hash. Defaults to `Buf32::zero()`.
    #[serde(default = "Buf32::zero")]
    pub logs_root: Buf32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_all_defaults() {
        let json = r#"{}"#;
        let params = serde_json::from_str::<HeaderParams>(json).expect("parse failed");

        assert_eq!(params.timestamp, 0);
        assert_eq!(params.epoch, 0);
        assert_eq!(params.parent_blkid, OLBlockId::null());
        assert_eq!(params.body_root, Buf32::zero());
        assert_eq!(params.logs_root, Buf32::zero());
    }

    #[test]
    fn test_header_explicit_values() {
        let json = r#"{
            "timestamp": 42,
            "epoch": 7,
            "parent_blkid": "0101010101010101010101010101010101010101010101010101010101010101",
            "body_root": "0202020202020202020202020202020202020202020202020202020202020202",
            "logs_root": "0303030303030303030303030303030303030303030303030303030303030303"
        }"#;
        let params = serde_json::from_str::<HeaderParams>(json).expect("parse failed");

        assert_eq!(params.timestamp, 42);
        assert_eq!(params.epoch, 7);
        assert_eq!(
            params.parent_blkid,
            OLBlockId::from(Buf32::from([0x01; 32]))
        );
        assert_eq!(params.body_root, Buf32::from([0x02; 32]));
        assert_eq!(params.logs_root, Buf32::from([0x03; 32]));
    }

    #[test]
    fn test_header_partial_defaults() {
        let json = r#"{ "timestamp": 100 }"#;
        let params = serde_json::from_str::<HeaderParams>(json).expect("parse failed");

        assert_eq!(params.timestamp, 100);
        assert_eq!(params.epoch, 0);
        assert_eq!(params.parent_blkid, OLBlockId::null());
        assert_eq!(params.body_root, Buf32::zero());
        assert_eq!(params.logs_root, Buf32::zero());
    }

    #[test]
    fn test_header_json_roundtrip() {
        let json = r#"{
            "timestamp": 10,
            "epoch": 3,
            "parent_blkid": "abababababababababababababababababababababababababababababababab",
            "body_root": "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
            "logs_root": "efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef"
        }"#;
        let params = serde_json::from_str::<HeaderParams>(json).expect("parse failed");
        let serialized = serde_json::to_string(&params).expect("serialization failed");
        let decoded =
            serde_json::from_str::<HeaderParams>(&serialized).expect("deserialization failed");

        assert_eq!(params.timestamp, decoded.timestamp);
        assert_eq!(params.epoch, decoded.epoch);
        assert_eq!(params.parent_blkid, decoded.parent_blkid);
        assert_eq!(params.body_root, decoded.body_root);
        assert_eq!(params.logs_root, decoded.logs_root);
    }
}
