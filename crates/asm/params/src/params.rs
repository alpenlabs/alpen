#[cfg(feature = "arbitrary")]
use arbitrary::{Arbitrary, Unstructured};
use serde::{Deserialize, Serialize, de::Error};
use strata_btc_types::GenesisL1View;
use strata_l1_txfmt::MagicBytes;

use crate::subprotocols::SubprotocolInstance;

/// Top-level parameters for an ASM instance.
///
/// Combines the SPS-50 magic bytes used to tag L1 transactions, the genesis
/// L1 view that bootstraps header verification, and the set of active
/// subprotocol configurations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsmParams {
    /// SPS-50 magic bytes that identify protocol transactions on L1.
    #[serde(with = "serde_magic_bytes")]
    pub magic: MagicBytes,

    /// Genesis L1 view used to bootstrap PoW header verification.
    pub l1_view: GenesisL1View,

    /// Ordered list of subprotocol configurations active in this ASM.
    pub subprotocols: Vec<SubprotocolInstance>,
}

/// Serialize/deserialize [`MagicBytes`] as a human-readable string using its
/// Display/FromStr implementation.
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

#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for AsmParams {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        use strata_btc_types::TIMESTAMPS_FOR_MEDIAN;
        use strata_identifiers::L1BlockCommitment;

        use crate::subprotocols::{AdministrationSubprotoParams, BridgeV1Config, CheckpointConfig};

        let blk = L1BlockCommitment::arbitrary(u)?;
        let l1_view = GenesisL1View {
            blk,
            next_target: u.arbitrary()?,
            epoch_start_timestamp: u.arbitrary()?,
            last_11_timestamps: u.arbitrary::<[u32; TIMESTAMPS_FOR_MEDIAN]>()?,
        };

        Ok(Self {
            magic: MagicBytes::new(*b"ALPN"),
            l1_view,
            subprotocols: vec![
                SubprotocolInstance::Admin(AdministrationSubprotoParams::arbitrary(u)?),
                SubprotocolInstance::Checkpoint(CheckpointConfig::arbitrary(u)?),
                SubprotocolInstance::Bridge(BridgeV1Config::arbitrary(u)?),
            ],
        })
    }
}
