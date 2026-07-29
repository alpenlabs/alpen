use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
use strata_acct_types::AccountId;
use strata_bridge_params::BridgeParams;

/// Default Alpen EE account id registered in generated OL params.
pub const DEFAULT_ALPEN_EE_ACCOUNT_ID: AccountId = AccountId::new([1u8; 32]);

/// Chain specific config, that needs to remain constant on all nodes
/// to ensure all stay on the same chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlpenEeParams {
    /// Account id of current EE in OL
    account_id: AccountId,

    /// Genesis blockhash of execution chain
    genesis_blockhash: B256,

    /// Genesis stateroot of execution chain
    genesis_stateroot: B256,

    /// Block number of execution chain genesis block
    /// This can potentially be non-zero, but is very unlikely.
    genesis_blocknum: u64,

    /// Bridge denomination and withdrawal policy.
    bridge_params: BridgeParams,
}

impl AlpenEeParams {
    /// Creates new chain parameters.
    pub fn new(
        account_id: AccountId,
        genesis_blockhash: B256,
        genesis_stateroot: B256,
        genesis_blocknum: u64,
        bridge_params: BridgeParams,
    ) -> Self {
        Self {
            account_id,
            genesis_blockhash,
            genesis_stateroot,
            genesis_blocknum,
            bridge_params,
        }
    }

    /// Parses chain parameters from a JSON string.
    pub fn from_json_str(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Serializes chain parameters to pretty-printed JSON.
    pub fn to_json_string_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Returns the EE account ID in the OL chain.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Returns the genesis block hash of the execution chain.
    pub fn genesis_blockhash(&self) -> B256 {
        self.genesis_blockhash
    }

    /// Returns the genesis state root of the execution chain.
    pub fn genesis_stateroot(&self) -> B256 {
        self.genesis_stateroot
    }

    /// Returns the genesis block number of the execution chain.
    pub fn genesis_blocknum(&self) -> u64 {
        self.genesis_blocknum
    }

    /// Returns the bridge denomination and withdrawal policy.
    pub fn bridge_params(&self) -> &BridgeParams {
        &self.bridge_params
    }
}

#[cfg(test)]
mod tests {
    use strata_bridge_params::BridgeParams;

    use super::{AlpenEeParams, DEFAULT_ALPEN_EE_ACCOUNT_ID};

    #[test]
    fn json_roundtrip_preserves_params() {
        let params = AlpenEeParams::new(
            DEFAULT_ALPEN_EE_ACCOUNT_ID,
            [2u8; 32].into(),
            [3u8; 32].into(),
            42,
            BridgeParams::new(100_000_000, Some(1_000_000_000)).expect("valid bridge params"),
        );

        let json = params
            .to_json_string_pretty()
            .expect("params should serialize");
        let decoded = AlpenEeParams::from_json_str(&json).expect("params should deserialize");

        assert_eq!(decoded, params);
    }

    #[test]
    fn json_rejects_malformed_account_id() {
        let json = r#"{
            "account_id": "01",
            "genesis_blockhash": "0x0202020202020202020202020202020202020202020202020202020202020202",
            "genesis_stateroot": "0x0303030303030303030303030303030303030303030303030303030303030303",
            "genesis_blocknum": 0,
            "bridge_params": {
                "denomination": 100000000,
                "max_withdrawal_amount": 1000000000,
                "max_withdrawal_descriptor_len": 81
            }
        }"#;

        assert!(AlpenEeParams::from_json_str(json).is_err());
    }

    #[test]
    fn json_rejects_missing_bridge_params() {
        let json = r#"{
            "account_id": "0101010101010101010101010101010101010101010101010101010101010101",
            "genesis_blockhash": "0x0202020202020202020202020202020202020202020202020202020202020202",
            "genesis_stateroot": "0x0303030303030303030303030303030303030303030303030303030303030303",
            "genesis_blocknum": 0
        }"#;

        assert!(AlpenEeParams::from_json_str(json).is_err());
    }
}
