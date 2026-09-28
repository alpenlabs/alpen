use std::collections::BTreeMap;

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ssz::Encode;
use ssz_derive::{Decode, Encode};
use strata_identifiers::{AccountId, EpochCommitment, L1BlockCommitment};
use strata_ol_state_types::OLSpecId;

use crate::{BridgeParams, GenesisHeaderParams, GenesisSnarkAccountData};

/// OL genesis parameters.
///
/// These fields are needed to construct genesis state and do not need to be
/// embedded into proof programs after genesis initialization.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLGenesisParams {
    /// Spec the genesis block runs under, which is the network's first spec.
    ///
    /// Required, with no default: a missing consensus parameter must not be
    /// guessed. Networks launched on 0.3.0 started under [`OLSpecId::V0`].
    #[serde(with = "spec_id_serde")]
    spec: OLSpecId,

    /// Header parameters for the parent of the genesis block.
    #[serde(default)]
    header: GenesisHeaderParams,

    /// Genesis accounts keyed by account ID.
    #[serde(default)]
    accounts: BTreeMap<AccountId, GenesisSnarkAccountData>,

    /// Last L1 block known at genesis time, treated as the initial verified L1 tip.
    #[serde(default)]
    last_l1_block: L1BlockCommitment,
}

impl OLGenesisParams {
    fn new(
        spec: OLSpecId,
        header: GenesisHeaderParams,
        accounts: BTreeMap<AccountId, GenesisSnarkAccountData>,
        last_l1_block: L1BlockCommitment,
    ) -> Self {
        Self {
            spec,
            header,
            accounts,
            last_l1_block,
        }
    }

    /// Returns the spec the genesis block runs under.
    pub fn spec(&self) -> OLSpecId {
        self.spec
    }

    pub fn header(&self) -> &GenesisHeaderParams {
        &self.header
    }

    pub fn accounts(&self) -> &BTreeMap<AccountId, GenesisSnarkAccountData> {
        &self.accounts
    }

    pub fn last_l1_block(&self) -> L1BlockCommitment {
        self.last_l1_block
    }
}

/// OL runtime parameters.
///
/// These fields affect OL STF execution and therefore must be bound to proof
/// artifacts when the STF runs inside a zkVM guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLRuntimeParams {
    /// Withdrawal denomination and optional cap.
    bridge_params: BridgeParams,
}

impl OLRuntimeParams {
    pub fn new(bridge_params: BridgeParams) -> Self {
        Self { bridge_params }
    }

    #[cfg(any(test, feature = "test-defaults"))]
    pub fn test_default() -> Self {
        Self::new(BridgeParams::default())
    }

    pub fn bridge_params(&self) -> &BridgeParams {
        &self.bridge_params
    }

    /// Computes the SHA-256 hash of the SSZ-encoded runtime params.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.as_ssz_bytes()).into()
    }
}

/// Top-level OL params file.
///
/// This type separates genesis-only inputs from runtime parameters that are
/// needed when executing the OL STF.
///
/// # File layout
///
/// The file is `{"genesis": {"spec": .., ..}, "runtime": {..}}`, and unknown
/// top-level fields are rejected. 0.3.0 wrote a flat layout with `header`,
/// `accounts`, `last_l1_block` and `bridge_params` at the top level, so such a
/// file fails to parse. Converting it keeps every value and sets the genesis
/// spec to [`OLSpecId::V0`], with `bridge_params` moving under `runtime`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLParams {
    /// Params used to construct OL genesis state.
    genesis: OLGenesisParams,

    /// Params used while executing the OL STF.
    runtime: OLRuntimeParams,
}

impl OLParams {
    /// Starts building [`OLParams`] with explicit runtime params.
    pub fn builder(runtime: OLRuntimeParams) -> OLParamsBuilder {
        OLParamsBuilder::new(runtime)
    }

    fn new(genesis: OLGenesisParams, runtime: OLRuntimeParams) -> Self {
        Self { genesis, runtime }
    }

    #[cfg(any(test, feature = "test-defaults"))]
    pub fn test_default() -> Self {
        Self::builder(OLRuntimeParams::test_default()).build()
    }

    /// Extracts the genesis-only portion of these params.
    pub fn genesis_params(&self) -> &OLGenesisParams {
        &self.genesis
    }

    /// Extracts the runtime portion of these params.
    pub fn runtime_params(&self) -> OLRuntimeParams {
        self.runtime
    }

    /// Returns the spec the genesis block runs under, which is the network's
    /// first spec.
    pub fn genesis_spec(&self) -> OLSpecId {
        self.genesis.spec
    }

    pub fn bridge_params(&self) -> &BridgeParams {
        self.runtime.bridge_params()
    }

    /// Returns the L1 block commitment used as OL genesis anchor.
    pub fn genesis_l1_block(&self) -> L1BlockCommitment {
        self.genesis.last_l1_block
    }

    /// Builds an [`EpochCommitment`] from the genesis header parameters.
    ///
    /// The genesis header's epoch, slot, and parent block ID are treated as a
    /// checkpointed epoch, serving as the initial verified commitment.
    pub fn derive_genesis_epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment::new(
            self.genesis.header.epoch,
            self.genesis.header.slot,
            self.genesis.header.parent_blkid,
        )
    }
}

/// Builder for assembling immutable [`OLParams`].
#[derive(Clone, Debug)]
pub struct OLParamsBuilder {
    genesis: OLGenesisParams,
    runtime: OLRuntimeParams,
}

impl OLParamsBuilder {
    /// Starts params for a new network, whose genesis runs under
    /// [`OLSpecId::V1`]: this release never produces blocks under older rules.
    pub fn new(runtime: OLRuntimeParams) -> Self {
        Self {
            genesis: OLGenesisParams::new(
                OLSpecId::V1,
                GenesisHeaderParams::default(),
                BTreeMap::new(),
                L1BlockCommitment::default(),
            ),
            runtime,
        }
    }

    pub fn genesis_spec(mut self, spec: OLSpecId) -> Self {
        self.genesis.spec = spec;
        self
    }

    pub fn genesis_header(mut self, header: GenesisHeaderParams) -> Self {
        self.genesis.header = header;
        self
    }

    pub fn genesis_l1_block(mut self, last_l1_block: L1BlockCommitment) -> Self {
        self.genesis.last_l1_block = last_l1_block;
        self
    }

    pub fn genesis_accounts(
        mut self,
        accounts: BTreeMap<AccountId, GenesisSnarkAccountData>,
    ) -> Self {
        self.genesis.accounts = accounts;
        self
    }

    pub fn build(self) -> OLParams {
        OLParams::new(self.genesis, self.runtime)
    }
}

/// Serializes an [`OLSpecId`] as its numeric discriminant, which is also its
/// SSZ `uint8` value.
mod spec_id_serde {
    use serde::de::{self, Unexpected};
    use serde::{Deserialize, Deserializer, Serializer};
    use strata_identifiers::SszDelegate;
    use strata_ol_state_types::OLSpecId;

    pub(super) fn serialize<S: Serializer>(
        spec: &OLSpecId,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(spec.into_delegate())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<OLSpecId, D::Error> {
        let raw = u8::deserialize(deserializer)?;
        OLSpecId::from_delegate(raw).map_err(|_| {
            de::Error::invalid_value(
                Unexpected::Unsigned(raw.into()),
                &"a known OL spec identifier",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use strata_btc_types::BitcoinAmount;
    use strata_identifiers::{Buf32, OLBlockId};
    use strata_predicate::PredicateKey;

    use super::*;

    /// Params file shaped like MN0's, converted from the flat 0.3.0 layout.
    const CONVERTED_V0_PARAMS: &str = r#"{
        "genesis": {
            "spec": 0,
            "header": {
                "timestamp": 11,
                "slot": 12,
                "epoch": 13,
                "parent_blkid": "1414141414141414141414141414141414141414141414141414141414141414",
                "body_root": "1515151515151515151515151515151515151515151515151515151515151515",
                "logs_root": "1616161616161616161616161616161616161616161616161616161616161616"
            },
            "accounts": {
                "0101010101010101010101010101010101010101010101010101010101010101": {
                    "predicate": "AlwaysAccept",
                    "inner_state": "308ce726a90fd45d3638fd86dec816cca262edc0d5acee9b130cfa33dbb740b0",
                    "balance": 0
                }
            },
            "last_l1_block": {
                "height": 961729,
                "blkid": "0000000000000000000055fbd96192d25981163ea0f18accc6d1e22cafe84b6f"
            }
        },
        "runtime": {
            "bridge_params": {
                "denomination": 200000000,
                "max_withdrawal_amount": null,
                "max_withdrawal_descriptor_len": 81
            }
        }
    }"#;

    /// A params file in the flat layout 0.3.0 wrote.
    const FLAT_V030_PARAMS: &str = r#"{
        "header": {},
        "accounts": {},
        "last_l1_block": {
            "height": 961729,
            "blkid": "0000000000000000000055fbd96192d25981163ea0f18accc6d1e22cafe84b6f"
        },
        "bridge_params": {
            "denomination": 200000000,
            "max_withdrawal_amount": null,
            "max_withdrawal_descriptor_len": 81
        }
    }"#;

    fn nested_params(spec: &str) -> String {
        format!(
            r#"{{
                "genesis": {{
                    {spec}
                    "header": {{}},
                    "accounts": {{}},
                    "last_l1_block": {{
                        "height": 0,
                        "blkid": "0000000000000000000000000000000000000000000000000000000000000000"
                    }}
                }},
                "runtime": {{
                    "bridge_params": {{
                        "denomination": 100000000,
                        "max_withdrawal_amount": 1000000000,
                        "max_withdrawal_descriptor_len": 81
                    }}
                }}
            }}"#
        )
    }

    /// Asserts that parsing fails with a message containing `expected` and a
    /// position, which `strata` needs to render the error at startup.
    fn assert_parse_error(json: &str, expected: &str) {
        let err = serde_json::from_str::<OLParams>(json).expect_err("params must not parse");
        assert!(
            err.to_string().contains(expected),
            "error {err} does not mention {expected}"
        );
        assert!(err.line() > 0, "error {err} has no position");
    }

    fn sample_params() -> OLParams {
        OLParams::test_default()
    }

    #[test]
    fn split_params_use_nested_json_shape() {
        let params = sample_params();
        let json = serde_json::to_value(&params).expect("serialization failed");

        assert!(json.get("genesis").is_some());
        assert!(json.get("runtime").is_some());
        assert!(json.get("header").is_none());
        assert!(json.get("accounts").is_none());
        assert!(json.get("last_l1_block").is_none());
        assert!(json.get("bridge_params").is_none());
    }

    #[test]
    fn builder_starts_new_networks_at_v1() {
        let params = sample_params();
        assert_eq!(params.genesis_spec(), OLSpecId::V1);

        let json = serde_json::to_value(&params).expect("serialization failed");
        assert_eq!(json["genesis"]["spec"], 1);

        let v0 = OLParams::builder(OLRuntimeParams::test_default())
            .genesis_spec(OLSpecId::V0)
            .build();
        assert_eq!(v0.genesis_spec(), OLSpecId::V0);
    }

    #[test]
    fn missing_runtime_params_errors() {
        let json = r#"{
            "genesis": {
                "spec": 1,
                "header": {},
                "accounts": {},
                "last_l1_block": {
                    "height": 0,
                    "blkid": "0000000000000000000000000000000000000000000000000000000000000000"
                }
            }
        }"#;

        assert_parse_error(json, "missing field `runtime`");
    }

    #[test]
    fn genesis_spec_reads_as_its_discriminant() {
        for (raw, spec) in [(0, OLSpecId::V0), (1, OLSpecId::V1)] {
            let params =
                serde_json::from_str::<OLParams>(&nested_params(&format!(r#""spec": {raw},"#)))
                    .expect("nested params parse");
            assert_eq!(params.genesis_spec(), spec);
        }
    }

    #[test]
    fn genesis_spec_is_required() {
        assert_parse_error(&nested_params(""), "missing field `spec`");
    }

    #[test]
    fn unknown_genesis_spec_errors() {
        assert_parse_error(
            &nested_params(r#""spec": 7,"#),
            "expected a known OL spec identifier",
        );
        assert_parse_error(&nested_params(r#""spec": "V1","#), "expected u8");
    }

    #[test]
    fn flat_v030_layout_errors() {
        assert_parse_error(FLAT_V030_PARAMS, "unknown field `header`");
    }

    #[test]
    fn stray_flat_field_errors() {
        let json = nested_params(r#""spec": 1,"#).replacen(
            r#""runtime": {"#,
            r#""bridge_params": {
                "denomination": 1,
                "max_withdrawal_amount": null,
                "max_withdrawal_descriptor_len": 81
            },
            "runtime": {"#,
            1,
        );
        assert_parse_error(&json, "unknown field `bridge_params`");
    }

    #[test]
    fn converted_v0_params_round_trip() {
        let params =
            serde_json::from_str::<OLParams>(CONVERTED_V0_PARAMS).expect("converted params parse");
        let genesis = params.genesis_params();

        assert_eq!(params.genesis_spec(), OLSpecId::V0);
        assert_eq!(genesis.header().slot, 12);
        assert_eq!(
            genesis.header().parent_blkid,
            OLBlockId::from(Buf32::from([0x14; 32]))
        );
        let account = &genesis.accounts()[&AccountId::from([0x01; 32])];
        assert_eq!(account.predicate, PredicateKey::always_accept());
        assert_eq!(account.balance, BitcoinAmount::default());
        assert_eq!(genesis.last_l1_block().height(), 961_729);
        assert_eq!(params.bridge_params().max_withdrawal_amount(), None);

        let input: serde_json::Value =
            serde_json::from_str(CONVERTED_V0_PARAMS).expect("fixture is JSON");
        assert_eq!(
            serde_json::to_value(&params).expect("serialize params"),
            input
        );
    }
}
