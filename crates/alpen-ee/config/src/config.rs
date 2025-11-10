use std::sync::Arc;

use strata_identifiers::CredRule;

use crate::{defaults::DEFAULT_DB_RETRY_COUNT, AlpenEeParams};

/// Local config that may differ between nodes + params.
#[derive(Debug, Clone)]
pub struct AlpenEeConfig {
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
    pub fn new(
        params: AlpenEeParams,
        sequencer_credrule: CredRule,
        ol_client_http: String,
        ee_sequencer_http: Option<String>,
        db_retry_count: Option<u16>,
    ) -> Self {
        Self {
            params: Arc::new(params),
            sequencer_credrule,
            ol_client_http,
            ee_sequencer_http,
            db_retry_count: db_retry_count.unwrap_or(DEFAULT_DB_RETRY_COUNT),
        }
    }

    pub fn params(&self) -> &Arc<AlpenEeParams> {
        &self.params
    }

    pub fn sequencer_credrule(&self) -> &CredRule {
        &self.sequencer_credrule
    }

    pub fn ol_client_http(&self) -> &str {
        &self.ol_client_http
    }

    pub fn ee_sequencer_http(&self) -> Option<&str> {
        self.ee_sequencer_http.as_deref()
    }

    pub fn db_retry_count(&self) -> u16 {
        self.db_retry_count
    }
}
