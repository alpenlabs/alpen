use std::fmt;

use hex_literal::hex;
use strata_asm_common::SubprotocolId;

/// Unique identifier for the Administration Subprotocol.
pub const ADMINISTRATION_SUBPROTOCOL_ID: SubprotocolId = 0;

/// Administration subprotocol transaction types.
///
/// This enum represents all valid transaction types for the Administration subprotocol.
/// Each variant corresponds to a specific transaction type with its associated u8 value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AdminTxType {
    /// Cancel a previously queued update.
    Cancel = 0,
    /// Update the strata admin multisignature configuration.
    StrataAdminMultisigUpdate = 10,
    /// Update the strata seq manager multisignature configuration.
    StrataSeqManagerMultisigUpdate = 11,
    /// Update the set of authorized operators.
    OperatorUpdate = 20,
    /// Update the sequencer configuration.
    SequencerUpdate = 21,
    /// Update the verifying key for the OL STF.
    OlStfVkUpdate = 30,
    /// Update the verifying key for the ASM STF.
    AsmStfVkUpdate = 31,
}

impl From<AdminTxType> for u8 {
    fn from(tx_type: AdminTxType) -> Self {
        tx_type as u8
    }
}

impl TryFrom<u8> for AdminTxType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AdminTxType::Cancel),
            10 => Ok(AdminTxType::StrataAdminMultisigUpdate),
            11 => Ok(AdminTxType::StrataSeqManagerMultisigUpdate),
            20 => Ok(AdminTxType::OperatorUpdate),
            21 => Ok(AdminTxType::SequencerUpdate),
            30 => Ok(AdminTxType::OlStfVkUpdate),
            31 => Ok(AdminTxType::AsmStfVkUpdate),
            invalid => Err(invalid),
        }
    }
}

impl AdminTxType {
    /// Returns the precomputed `SHA256(tag)` for this transaction type's
    /// sighash tag, where `tag = "strata/admin/<tx_type_name>"`.
    pub fn sighash_tag_hash(&self) -> &'static [u8; 32] {
        match self {
            // SHA256("strata/admin/cancel")
            Self::Cancel => {
                &hex!("35d7714c91591bcdd57783d64211bff38b471d09e2d3c941c3631b3a4083d64e")
            }
            // SHA256("strata/admin/strata_admin_multisig_update")
            Self::StrataAdminMultisigUpdate => {
                &hex!("020eaac546220fde8c6bb34b249c36600328a57aa886c8eee36dfa939ac14e1b")
            }
            // SHA256("strata/admin/strata_seq_manager_multisig_update")
            Self::StrataSeqManagerMultisigUpdate => {
                &hex!("0134b3ef6be62aa4d34cc93aa5f2e89ffdc3dec7f615c147c2d5e45667a500a9")
            }
            // SHA256("strata/admin/operator_update")
            Self::OperatorUpdate => {
                &hex!("7beec647ba1cf0122848227f51d04feb247d1343a626f9cbd78a1d6f30c8b908")
            }
            // SHA256("strata/admin/sequencer_update")
            Self::SequencerUpdate => {
                &hex!("81eaf3408d1a3c84865299143508f96dead2c4a495ee562ba4419ad32d1ff43b")
            }
            // SHA256("strata/admin/ol_stf_vk_update")
            Self::OlStfVkUpdate => {
                &hex!("5078d7aeaac88a527b39fd491e351ea10d85fd22888e21744bb9913d342a1120")
            }
            // SHA256("strata/admin/asm_stf_vk_update")
            Self::AsmStfVkUpdate => {
                &hex!("3b997494e33fb473fd81869e0128ff44489e8ee2b07a791ab157514672d36ced")
            }
        }
    }
}

impl fmt::Display for AdminTxType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdminTxType::Cancel => write!(f, "Cancel"),
            AdminTxType::StrataAdminMultisigUpdate => write!(f, "StrataAdminMultisigUpdate"),
            AdminTxType::StrataSeqManagerMultisigUpdate => {
                write!(f, "StrataSeqManagerMultisigUpdate")
            }
            AdminTxType::OperatorUpdate => write!(f, "OperatorUpdate"),
            AdminTxType::SequencerUpdate => write!(f, "SequencerUpdate"),
            AdminTxType::OlStfVkUpdate => write!(f, "OlStfVkUpdate"),
            AdminTxType::AsmStfVkUpdate => write!(f, "AsmStfVkUpdate"),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_crypto::hash;

    use super::*;

    impl Arbitrary for AdminTxType {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(AdminTxType::Cancel),
                Just(AdminTxType::StrataAdminMultisigUpdate),
                Just(AdminTxType::StrataSeqManagerMultisigUpdate),
                Just(AdminTxType::OperatorUpdate),
                Just(AdminTxType::SequencerUpdate),
                Just(AdminTxType::OlStfVkUpdate),
                Just(AdminTxType::AsmStfVkUpdate),
            ]
            .boxed()
        }
    }

    #[test]
    fn test_admin_tx_type_discriminants() {
        assert_eq!(AdminTxType::Cancel as u8, 0);
        assert_eq!(AdminTxType::StrataAdminMultisigUpdate as u8, 10);
        assert_eq!(AdminTxType::StrataSeqManagerMultisigUpdate as u8, 11);
        assert_eq!(AdminTxType::OperatorUpdate as u8, 20);
        assert_eq!(AdminTxType::SequencerUpdate as u8, 21);
        assert_eq!(AdminTxType::OlStfVkUpdate as u8, 30);
        assert_eq!(AdminTxType::AsmStfVkUpdate as u8, 31);
    }

    /// Verifies that each hardcoded sighash tag constant equals
    /// `SHA256("strata/admin/<tag_name>")`.
    #[test]
    fn test_sighash_tag_hashes_match_sha256() {
        let cases: &[(AdminTxType, &str)] = &[
            (AdminTxType::Cancel, "strata/admin/cancel"),
            (
                AdminTxType::StrataAdminMultisigUpdate,
                "strata/admin/strata_admin_multisig_update",
            ),
            (
                AdminTxType::StrataSeqManagerMultisigUpdate,
                "strata/admin/strata_seq_manager_multisig_update",
            ),
            (AdminTxType::OperatorUpdate, "strata/admin/operator_update"),
            (
                AdminTxType::SequencerUpdate,
                "strata/admin/sequencer_update",
            ),
            (AdminTxType::OlStfVkUpdate, "strata/admin/ol_stf_vk_update"),
            (
                AdminTxType::AsmStfVkUpdate,
                "strata/admin/asm_stf_vk_update",
            ),
        ];

        for (tx_type, tag) in cases {
            let expected = hash::raw(tag.as_bytes()).0;
            assert_eq!(
                tx_type.sighash_tag_hash(),
                &expected,
                "sighash tag hash mismatch for {tx_type} (tag: {tag:?})"
            );
        }
    }

    proptest! {
        #[test]
        fn test_admin_tx_type_roundtrip(tx_type: AdminTxType) {
            let as_u8: u8 = tx_type.into();
            let back_to_enum = AdminTxType::try_from(as_u8)
                .expect("roundtrip conversion should succeed");
            prop_assert_eq!(tx_type, back_to_enum);
        }

        #[test]
        fn test_admin_tx_type_invalid_values(
            value in (0u8..=255u8).prop_filter("must not be a valid variant", |v| {
                !matches!(*v, 0 | 10 | 11 | 20 | 21 | 30 | 31)
            })
        ) {
            prop_assert!(AdminTxType::try_from(value).is_err());
        }
    }
}
