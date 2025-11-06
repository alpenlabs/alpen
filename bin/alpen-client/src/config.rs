use std::sync::Arc;

use alloy_primitives::B256;
use strata_acct_types::AccountId;
use strata_identifiers::{CredRule, OLBlockId};

/// Chain specific config, that needs to remain constant on all nodes
/// to ensure all stay on the same chain.
#[derive(Debug, Clone)]
pub(crate) struct AlpenEeParams {
    /// Account id of current EE in OL
    account_id: AccountId,

    /// Genesis blockhash of execution chain
    genesis_blockhash: B256,

    /// Genesis stateroot of execution chain
    genesis_stateroot: B256,

    /// OL slot of Alpen ee account genesis
    genesis_ol_slot: u64,

    /// Ol block of Alpen ee account genesis
    genesis_ol_blockid: OLBlockId,
}

impl AlpenEeParams {
    pub(crate) fn new(
        account_id: AccountId,
        genesis_blockhash: B256,
        genesis_stateroot: B256,
        genesis_ol_slot: u64,
        genesis_ol_blockid: OLBlockId,
    ) -> Self {
        Self {
            account_id,
            genesis_blockhash,
            genesis_stateroot,
            genesis_ol_slot,
            genesis_ol_blockid,
        }
    }

    #[expect(dead_code, reason = "wip")]
    pub(crate) fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub(crate) fn genesis_blockhash(&self) -> B256 {
        self.genesis_blockhash
    }

    #[expect(dead_code, reason = "wip")]
    pub(crate) fn genesis_stateroot(&self) -> B256 {
        self.genesis_stateroot
    }

    pub(crate) fn genesis_ol_slot(&self) -> u64 {
        self.genesis_ol_slot
    }

    pub(crate) fn genesis_ol_blockid(&self) -> OLBlockId {
        self.genesis_ol_blockid
    }
}

/// Local config that may differ between nodes + params.
#[derive(Debug, Clone)]
pub(crate) struct AlpenEeConfig {
    /// Chain specific config.
    params: Arc<AlpenEeParams>,

    /// To verify preconfirmed updates from sequencer.
    sequencer_credrule: CredRule,

    /// Connection OL RPC client.
    ol_client_http: String,

    /// Connection EE sequencer client.
    ee_sequencer_http: Option<String>,

    /// Number of retries for db connections
    db_retry_count: u16,
}

impl AlpenEeConfig {
    pub(crate) fn new(
        params: AlpenEeParams,
        sequencer_credrule: CredRule,
        ol_client_http: String,
        ee_sequencer_http: Option<String>,
        db_retry_count: u16,
    ) -> Self {
        Self {
            params: Arc::new(params),
            sequencer_credrule,
            ol_client_http,
            ee_sequencer_http,
            db_retry_count,
        }
    }

    pub(crate) fn params(&self) -> &Arc<AlpenEeParams> {
        &self.params
    }

    #[expect(dead_code, reason = "wip")]
    pub(crate) fn sequencer_credrule(&self) -> &CredRule {
        &self.sequencer_credrule
    }

    #[expect(dead_code, reason = "wip")]
    pub(crate) fn ol_client_http(&self) -> &str {
        &self.ol_client_http
    }

    #[expect(dead_code, reason = "wip")]
    pub(crate) fn ee_sequencer_http(&self) -> Option<&str> {
        self.ee_sequencer_http.as_deref()
    }

    pub(crate) fn db_retry_count(&self) -> u16 {
        self.db_retry_count
    }
}

pub(crate) mod defaults {
    pub(crate) const DB_RETRY_COUNT: u16 = 5;
}
