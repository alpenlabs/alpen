use alloy_primitives::B256;
use strata_acct_types::AccountId;
use strata_identifiers::{CredRule, OLBlockId};

/// Chain specific config, that needs to remain constant on all nodes
/// to ensure all stay on the same chain.
#[derive(Debug, Clone)]
pub(crate) struct AlpenEeParams {
    /// Account id of current EE in OL
    #[expect(dead_code, reason = "wip")]
    pub account_id: AccountId,

    /// Genesis blockhash of execution chain
    pub genesis_blockhash: B256,

    /// Genesis stateroot of execution chain
    #[expect(dead_code, reason = "wip")]
    pub genesis_stateroot: B256,

    /// OL slot of Alpen ee account genesis
    pub genesis_ol_slot: u64,

    /// Ol block of Alpen ee account genesis
    pub genesis_ol_blockid: OLBlockId,
}

/// Local config that may differ between nodes + params.
#[derive(Debug, Clone)]
pub(crate) struct AlpenEeConfig {
    /// Chain specific config.
    pub params: AlpenEeParams,

    /// To verify preconfirmed updates from sequencer.
    #[expect(dead_code, reason = "wip")]
    pub sequencer_credrule: CredRule,

    /// Connection OL RPC client.
    #[expect(dead_code, reason = "wip")]
    pub ol_client_http: String,

    /// Connection EE sequencer client.
    #[expect(dead_code, reason = "wip")]
    pub ee_sequencer_http: Option<String>,

    /// Number of retries for db connections
    pub db_retry_count: u16,
}

pub(crate) mod defaults {
    pub(crate) const DB_RETRY_COUNT: u16 = 5;
}
