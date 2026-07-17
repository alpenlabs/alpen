//! EE DA stream identity parameters.

use serde::{Deserialize, Serialize};
use strata_l1_txfmt::MagicBytes;

/// Identity of the EE DA stream on L1.
///
/// Identifies the SPS-51 commit transactions that carry EE DA blobs.
/// Consensus-relevant: all nodes must agree on it, but it is not expected to
/// change across forks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobSpec {
    /// `OP_RETURN` magic identifying the EE DA stream in the commit tx.
    ///
    /// Serialized as a 4-character string (e.g. `"ALPN"`) in JSON.
    magic_bytes: MagicBytes,
}

impl BlobSpec {
    /// Creates a new blob spec.
    pub fn new(magic_bytes: MagicBytes) -> Self {
        Self { magic_bytes }
    }

    /// Returns the DA stream magic bytes.
    pub fn magic_bytes(&self) -> MagicBytes {
        self.magic_bytes
    }
}

#[cfg(test)]
mod tests {
    use strata_l1_txfmt::MagicBytes;

    use super::BlobSpec;

    #[test]
    fn json_roundtrip_preserves_magic_bytes() {
        let spec = BlobSpec::new(MagicBytes::new(*b"ALPN"));

        let json = serde_json::to_string(&spec).expect("blob spec should serialize");
        assert_eq!(json, r#"{"magic_bytes":"ALPN"}"#);

        let decoded: BlobSpec = serde_json::from_str(&json).expect("blob spec should deserialize");
        assert_eq!(decoded, spec);
    }

    #[test]
    fn json_rejects_wrong_length_magic_bytes() {
        assert!(serde_json::from_str::<BlobSpec>(r#"{"magic_bytes":"ALP"}"#).is_err());
        assert!(serde_json::from_str::<BlobSpec>(r#"{"magic_bytes":"ALPEN"}"#).is_err());
    }

    #[test]
    fn json_rejects_unknown_fields() {
        let json = r#"{"magic_bytes":"ALPN","max_chunk_payload":395000}"#;
        assert!(serde_json::from_str::<BlobSpec>(json).is_err());
    }
}
