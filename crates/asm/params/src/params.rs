#[cfg(feature = "arbitrary")]
use arbitrary::{Arbitrary, Unstructured};
use serde::{Deserialize, Serialize};
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
    pub magic: MagicBytes,

    /// Genesis L1 view used to bootstrap PoW header verification.
    pub l1_view: GenesisL1View,

    /// Ordered list of subprotocol configurations active in this ASM.
    pub subprotocols: Vec<SubprotocolInstance>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asm_params_deserialize_from_raw_json() {
        // Static JSON generated from arbitrary instance with seed [0..256]
        let raw_json = r#"
{
  "magic": "ALPN",
  "l1_view": {
    "blk": {
      "height": 50462976,
      "blkid": "0405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20212223"
    },
    "next_target": 656811300,
    "epoch_start_timestamp": 724183336,
    "last_11_timestamps": [
      791555372,
      858927408,
      926299444,
      993671480,
      1061043516,
      1128415552,
      1195787588,
      1263159624,
      1330531660,
      1397903696,
      1465275732
    ]
  },
  "subprotocols": [
    {
      "Admin": {
        "strata_administrator": {
          "keys": [
            "02bedfa2fa42d906565519bee43875608a09e06640203a6c7a43569150c7cbe7c5"
          ],
          "threshold": 1
        },
        "strata_sequencer_manager": {
          "keys": [
            "03cf59a1a5ef092ced386f2651b610d3dd2cc6806bb74a8eab95c1f3b2f3d81772",
            "02343edde4a056e00af99aa49de60df03859d1b79ebbc4f3f6da8fbd0053565de3"
          ],
          "threshold": 1
        },
        "confirmation_depth": 144,
        "max_seqno_gap": 10
      }
    },
    {
      "Checkpoint": {
        "sequencer_predicate": "Sp1Groth16",
        "checkpoint_predicate": "AlwaysAccept",
        "genesis_l1_height": 3334849731,
        "genesis_ol_blkid": "c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6"
      }
    },
    {
      "Bridge": {
        "operators": [
          "02becdf7aab195ab0a42ba2f2eca5b7fa5a246267d802c627010e1672f08657f70"
        ],
        "denomination": 0,
        "assignment_duration": 0,
        "operator_fee": 0,
        "recovery_delay": 0
      }
    }
  ]
}
"#;

        let params: AsmParams =
            serde_json::from_str(raw_json).expect("deserialization from raw JSON should succeed");

        // Verify key fields
        assert_eq!(params.magic, MagicBytes::new(*b"ALPN"));
        assert_eq!(params.l1_view.blk.height_u32(), 50462976);
        assert_eq!(params.subprotocols.len(), 3);
    }
}
