//! Module for resolving fee rates for transactions, supporting multiple fee policies including
//! Bitcoin Core's `estimatesmartfee` and mempool.space's recommended fees endpoint.

use std::{sync::LazyLock, time::Duration};

use bitcoin::FeeRate;
use bitcoind_async_client::{error::ClientError, traits::Reader};
use reqwest::{header::ACCEPT_ENCODING, Url};
use serde::Deserialize;
use strata_config::btcio::{
    fee_rate_from_sat_per_vb, FeePolicy, L1FeePolicyConfig, MempoolExplorerFeePolicy,
};
use thiserror::Error;
use tokio::time::timeout;
use tracing::warn;
use url::ParseError;

const DEFAULT_EXPLORER_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_BITCOIN_RPC_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_EXPLORER_RESPONSE_BYTES: usize = 16 * 1024;

/// Shared HTTP client reused across mempool fee lookups for connection pooling.
static SHARED_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Independent timeout budgets for external fee-rate lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeeRateResolutionTimeouts {
    explorer: Duration,
    bitcoin_rpc: Duration,
}

impl FeeRateResolutionTimeouts {
    /// Creates timeout budgets for the explorer phase and Bitcoin Core RPC phase.
    pub const fn new(explorer: Duration, bitcoin_rpc: Duration) -> Self {
        Self {
            explorer,
            bitcoin_rpc,
        }
    }

    /// Returns the total budget for the precise and recommended explorer requests.
    pub const fn explorer(self) -> Duration {
        self.explorer
    }

    /// Returns the budget for a Bitcoin Core fee-rate RPC request.
    pub const fn bitcoin_rpc(self) -> Duration {
        self.bitcoin_rpc
    }
}

impl Default for FeeRateResolutionTimeouts {
    fn default() -> Self {
        Self::new(DEFAULT_EXPLORER_TIMEOUT, DEFAULT_BITCOIN_RPC_TIMEOUT)
    }
}

/// Errors that can occur while resolving a Bitcoin fee rate.
#[derive(Debug, Error)]
pub enum FeeRateError {
    /// The mempool explorer phase exceeded its configured budget.
    #[error("mempool explorer fee lookup timed out after {timeout:?}")]
    ExplorerTimeout { timeout: Duration },

    /// Bitcoin Core's `estimatesmartfee` RPC exceeded its configured budget.
    #[error(
        "Bitcoin RPC fee lookup for confirmation target {conf_target} timed out after {timeout:?}"
    )]
    BitcoinRpcTimeout { conf_target: u16, timeout: Duration },

    /// Both the mempool explorer phase and its Bitcoin Core fallback failed.
    #[error("mempool explorer fee lookup failed ({explorer}); Bitcoin RPC fallback failed ({bitcoin_rpc})")]
    Fallback {
        explorer: Box<FeeRateError>,
        bitcoin_rpc: Box<FeeRateError>,
    },

    /// The configured mempool explorer URL is invalid.
    #[error("invalid mempool explorer configuration")]
    InvalidExplorerConfiguration {
        #[source]
        source: ParseError,
    },

    /// A mempool explorer request failed or returned an invalid response.
    #[error("invalid response from mempool explorer endpoint `{endpoint}`: {source}")]
    InvalidExplorerResponse {
        endpoint: &'static str,
        #[source]
        source: reqwest::Error,
    },

    /// A mempool explorer response exceeded the maximum accepted size.
    #[error("mempool explorer endpoint `{endpoint}` returned more than {max_bytes} bytes")]
    ExplorerResponseTooLarge {
        endpoint: &'static str,
        max_bytes: usize,
    },

    /// A mempool explorer response was not valid fee-estimate JSON.
    #[error("invalid JSON from mempool explorer endpoint `{endpoint}`: {source}")]
    InvalidExplorerJson {
        endpoint: &'static str,
        #[source]
        source: serde_json::Error,
    },

    /// A mempool explorer returned an invalid fee rate.
    #[error("invalid fee rate in mempool explorer response: {0}")]
    InvalidExplorerFeeRate(String),

    /// Bitcoin Core's `estimatesmartfee` RPC failed.
    #[error(
        "Bitcoin RPC failed while estimating the fee rate for confirmation target {conf_target}"
    )]
    BitcoinRpc {
        conf_target: u16,
        #[source]
        source: ClientError,
    },

    /// Bitcoin Core could not provide a smart-fee estimate.
    #[error("smart fee estimate unavailable for confirmation target {conf_target}: {errors:?}")]
    SmartFeeUnavailable {
        conf_target: u16,
        errors: Option<Vec<String>>,
    },
}

impl FeeRateError {
    /// Returns whether this error contains an expired explorer or Bitcoin RPC budget.
    pub fn is_timeout(&self) -> bool {
        match self {
            Self::ExplorerTimeout { .. } | Self::BitcoinRpcTimeout { .. } => true,
            Self::Fallback {
                explorer,
                bitcoin_rpc,
            } => explorer.is_timeout() || bitcoin_rpc.is_timeout(),
            Self::InvalidExplorerConfiguration { .. }
            | Self::InvalidExplorerResponse { .. }
            | Self::ExplorerResponseTooLarge { .. }
            | Self::InvalidExplorerJson { .. }
            | Self::InvalidExplorerFeeRate(_)
            | Self::BitcoinRpc { .. }
            | Self::SmartFeeUnavailable { .. } => false,
        }
    }
}

/// Represents the response from the mempool explorer recommended fees endpoint.
#[derive(Debug, Deserialize, PartialEq)]
pub(crate) struct MempoolRecommendedFees {
    #[serde(rename = "fastestFee")]
    fastest_fee: f64,
    #[serde(rename = "halfHourFee")]
    half_hour_fee: f64,
    #[serde(rename = "hourFee")]
    hour_fee: f64,
    #[serde(rename = "economyFee")]
    economy_fee: f64,
    #[serde(rename = "minimumFee")]
    minimum_fee: f64,
}

impl MempoolRecommendedFees {
    /// Selects the fee rate according to the given policy.
    fn select(self, policy: MempoolExplorerFeePolicy) -> Result<FeeRate, FeeRateError> {
        let fee_rate_sat_per_vb = match policy {
            MempoolExplorerFeePolicy::Fastest => self.fastest_fee,
            MempoolExplorerFeePolicy::HalfHour => self.half_hour_fee,
            MempoolExplorerFeePolicy::Hour => self.hour_fee,
            MempoolExplorerFeePolicy::Economy => self.economy_fee,
            MempoolExplorerFeePolicy::Minimum => self.minimum_fee,
        };
        fee_rate_from_sat_per_vb(fee_rate_sat_per_vb).map_err(FeeRateError::InvalidExplorerFeeRate)
    }
}

/// HTTP client for querying a mempool explorer's fee estimation API.
///
/// Reuses a module-level [`reqwest::Client`] for connection pooling across calls.
struct MempoolExplorerClient {
    base_url: Url,
}

impl MempoolExplorerClient {
    /// Creates a new client from a base URL string (e.g. `https://mempool.space/signet`).
    fn new(base_url: &str) -> Result<Self, FeeRateError> {
        let mut url = Url::parse(base_url)
            .map_err(|source| FeeRateError::InvalidExplorerConfiguration { source })?;

        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }

        Ok(Self { base_url: url })
    }

    /// Fetches fee estimates from a mempool.space-compatible endpoint path.
    ///
    /// The path is relative to the configured base URL so callers can try the precise endpoint
    /// first and fall back to the older recommended-fees endpoint when necessary.
    async fn fetch_fee_estimates(
        &self,
        path: &'static str,
    ) -> Result<MempoolRecommendedFees, FeeRateError> {
        let url = self
            .base_url
            .join(path)
            .map_err(|source| FeeRateError::InvalidExplorerConfiguration { source })?;

        let mut response = SHARED_HTTP_CLIENT
            .get(url)
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|source| FeeRateError::InvalidExplorerResponse {
                endpoint: path,
                source: source.without_url(),
            })?
            .error_for_status()
            .map_err(|source| FeeRateError::InvalidExplorerResponse {
                endpoint: path,
                source: source.without_url(),
            })?;

        let mut body = Vec::new();
        while let Some(chunk) =
            response
                .chunk()
                .await
                .map_err(|source| FeeRateError::InvalidExplorerResponse {
                    endpoint: path,
                    source: source.without_url(),
                })?
        {
            if body.len().saturating_add(chunk.len()) > MAX_EXPLORER_RESPONSE_BYTES {
                return Err(FeeRateError::ExplorerResponseTooLarge {
                    endpoint: path,
                    max_bytes: MAX_EXPLORER_RESPONSE_BYTES,
                });
            }
            body.extend_from_slice(&chunk);
        }

        serde_json::from_slice(&body).map_err(|source| FeeRateError::InvalidExplorerJson {
            endpoint: path,
            source,
        })
    }

    /// Fetches and selects a fee rate from the mempool explorer.
    async fn fetch_fee_rate(
        &self,
        policy: MempoolExplorerFeePolicy,
    ) -> Result<FeeRate, FeeRateError> {
        match self
            .fetch_fee_estimates("api/v1/fees/precise")
            .await
            .and_then(|fees| fees.select(policy))
        {
            Ok(fee_rate) => Ok(fee_rate),
            Err(err) => {
                warn!(
                    %err,
                    "mempool precise fee lookup failed, falling back to recommended endpoint"
                );
                self.fetch_fee_estimates("api/v1/fees/recommended")
                    .await?
                    .select(policy)
            }
        }
    }
}

/// Resolves the fee rate to use for a transaction based on the provided configuration.
pub async fn resolve_fee_rate<R: Reader>(
    client: &R,
    config: &L1FeePolicyConfig,
    timeouts: FeeRateResolutionTimeouts,
) -> Result<FeeRate, FeeRateError> {
    let fee_rate = match config.fee_policy() {
        FeePolicy::BitcoinD { conf_target } => {
            resolve_smart_fee_rate(client, *conf_target, timeouts.bitcoin_rpc()).await?
        }
        FeePolicy::MempoolExplorer {
            policy,
            mempool_base_url,
            fallback_conf_target,
        } => {
            resolve_mempool_fee_rate(
                client,
                mempool_base_url,
                *fallback_conf_target,
                *policy,
                timeouts,
            )
            .await?
        }
        FeePolicy::Fixed { fee_rate } => *fee_rate,
    };

    Ok(fee_rate)
}

/// Resolves the fee rate using the mempool explorer recommended fees endpoint, falling back to
/// Bitcoin Core's `estimatesmartfee` on failure.
async fn resolve_mempool_fee_rate<R: Reader>(
    client: &R,
    base_url: &str,
    fallback_conf_target: u16,
    mempool_fee_policy: MempoolExplorerFeePolicy,
    timeouts: FeeRateResolutionTimeouts,
) -> Result<FeeRate, FeeRateError> {
    let explorer = MempoolExplorerClient::new(base_url)?;

    let explorer_error = match timeout(
        timeouts.explorer(),
        explorer.fetch_fee_rate(mempool_fee_policy),
    )
    .await
    {
        Ok(Ok(fee_rate)) => return Ok(fee_rate),
        Ok(Err(err)) => err,
        Err(_) => FeeRateError::ExplorerTimeout {
            timeout: timeouts.explorer(),
        },
    };

    warn!(
        %explorer_error,
        fallback_conf_target,
        "mempool fee lookup failed, falling back to bitcoind's estimatesmartfee"
    );
    resolve_smart_fee_rate(client, fallback_conf_target, timeouts.bitcoin_rpc())
        .await
        .map_err(|bitcoin_rpc| FeeRateError::Fallback {
            explorer: Box::new(explorer_error),
            bitcoin_rpc: Box::new(bitcoin_rpc),
        })
}

async fn resolve_smart_fee_rate<R: Reader>(
    client: &R,
    conf_target: u16,
    request_timeout: Duration,
) -> Result<FeeRate, FeeRateError> {
    let estimate = timeout(request_timeout, client.estimate_smart_fee(conf_target))
        .await
        .map_err(|_| FeeRateError::BitcoinRpcTimeout {
            conf_target,
            timeout: request_timeout,
        })?
        .map_err(|source| FeeRateError::BitcoinRpc {
            conf_target,
            source,
        })?;

    estimate.fee_rate.ok_or(FeeRateError::SmartFeeUnavailable {
        conf_target,
        errors: estimate.errors,
    })
}

#[cfg(test)]
mod tests {
    use std::{error::Error, future::pending, time::Duration};

    use bitcoin::FeeRate;
    use bitcoind_async_client::error::ClientError;
    use strata_config::btcio::{FeePolicy, L1FeePolicyConfig, MempoolExplorerFeePolicy};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::{fee_rate_from_sat_per_vb, MempoolExplorerClient, MempoolRecommendedFees};
    use crate::{
        test_utils::TestBitcoinClient,
        writer::{resolve_fee_rate, FeeRateError, FeeRateResolutionTimeouts},
    };

    fn resolution_timeouts() -> FeeRateResolutionTimeouts {
        FeeRateResolutionTimeouts::new(Duration::from_secs(10), Duration::from_secs(10))
    }

    fn mempool_fee_config(policy: MempoolExplorerFeePolicy, base_url: String) -> L1FeePolicyConfig {
        L1FeePolicyConfig::new(FeePolicy::MempoolExplorer {
            policy,
            mempool_base_url: base_url,
            fallback_conf_target: 1,
        })
    }

    fn bitcoind_fee_config(conf_target: u16) -> L1FeePolicyConfig {
        L1FeePolicyConfig::new(FeePolicy::BitcoinD { conf_target })
    }

    async fn spawn_owned_response_server(responses: Vec<(&'static str, String)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        tokio::spawn(async move {
            for (status_line, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("accept should succeed");

                let mut buf = [0_u8; 1024];
                let _ = stream
                    .read(&mut buf)
                    .await
                    .expect("request read should succeed");
                let response = format!(
                    "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("response write should succeed");
            }
        });

        format!("http://{addr}")
    }

    async fn spawn_response_server(responses: Vec<(&'static str, &'static str)>) -> String {
        spawn_owned_response_server(
            responses
                .into_iter()
                .map(|(status, body)| (status, body.to_owned()))
                .collect(),
        )
        .await
    }

    async fn spawn_single_response_server(status_line: &'static str, body: &'static str) -> String {
        spawn_response_server(vec![(status_line, body)]).await
    }

    async fn spawn_stalled_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept should succeed");
            let mut request = [0_u8; 1024];
            let _bytes_read = stream
                .read(&mut request)
                .await
                .expect("request read should succeed");
            pending::<()>().await;
        });

        format!("http://{addr}")
    }

    #[test]
    fn test_mempool_recommended_fees_json_deserializes() {
        let json = r#"{
            "fastestFee": 1,
            "halfHourFee": 2,
            "hourFee": 3,
            "economyFee": 4,
            "minimumFee": 5
        }"#;

        let fees: MempoolRecommendedFees =
            serde_json::from_str(json).expect("response should deserialize");

        assert_eq!(
            fees,
            MempoolRecommendedFees {
                fastest_fee: 1.0,
                half_hour_fee: 2.0,
                hour_fee: 3.0,
                economy_fee: 4.0,
                minimum_fee: 5.0,
            }
        );
    }

    #[test]
    fn test_sat_per_vb_conversion_preserves_sub_sat_rates() {
        assert_eq!(
            fee_rate_from_sat_per_vb(0.5).expect("fee rate should convert"),
            FeeRate::from_sat_per_kwu(125)
        );
        assert_eq!(
            fee_rate_from_sat_per_vb(0.1).expect("fee rate should convert"),
            FeeRate::from_sat_per_kwu(25)
        );
    }

    #[test]
    fn test_mempool_explorer_client_normalizes_trailing_slash() {
        let without_slash =
            MempoolExplorerClient::new("https://mempool.space/signet").expect("url should parse");
        let with_slash =
            MempoolExplorerClient::new("https://mempool.space/signet/").expect("url should parse");

        assert_eq!(
            without_slash.base_url.as_str(),
            "https://mempool.space/signet/"
        );
        assert_eq!(without_slash.base_url, with_slash.base_url);
    }

    #[tokio::test]
    async fn test_mempool_explorer_response_error_strips_url_from_source_chain() {
        const SECRET: &str = "secret-bearing-password";

        let server = spawn_single_response_server("500 Internal Server Error", "").await;
        let credential_bearing_url =
            server.replacen("http://", &format!("http://user:{SECRET}@"), 1);
        let explorer =
            MempoolExplorerClient::new(&credential_bearing_url).expect("url should parse");

        let err = explorer
            .fetch_fee_estimates("api/v1/fees/precise")
            .await
            .expect_err("HTTP error should be returned");

        assert!(matches!(
            &err,
            FeeRateError::InvalidExplorerResponse {
                endpoint: "api/v1/fees/precise",
                source,
            } if source.url().is_none()
        ));

        let mut error_chain = Some(&err as &(dyn Error + 'static));
        while let Some(error) = error_chain {
            assert!(!error.to_string().contains(SECRET));
            assert!(!format!("{error:?}").contains(SECRET));
            error_chain = error.source();
        }
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_uses_precise_mempool_fee() {
        let server = spawn_single_response_server(
            "200 OK",
            "{\"fastestFee\":0.2,\"halfHourFee\":0.3,\"hourFee\":0.4,\"economyFee\":0.5,\"minimumFee\":0.1}",
        )
        .await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("mempool fee lookup should succeed");

        // 0.2 sat/vB, used as reported.
        assert_eq!(fee_rate, FeeRate::from_sat_per_kwu(50));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_uses_mempool_fastest_fee() {
        let server = spawn_single_response_server(
            "200 OK",
            "{\"fastestFee\":7,\"halfHourFee\":6,\"hourFee\":5,\"economyFee\":4,\"minimumFee\":3}",
        )
        .await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("mempool fee lookup should succeed");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(7));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_uses_selected_mempool_policy() {
        let server = spawn_single_response_server(
            "200 OK",
            "{\"fastestFee\":7,\"halfHourFee\":6,\"hourFee\":5,\"economyFee\":4,\"minimumFee\":3}",
        )
        .await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Economy, server);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("mempool fee lookup should succeed");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(4));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_from_precise_to_recommended_fees() {
        let server = spawn_response_server(vec![
            ("500 Internal Server Error", ""),
            (
                "200 OK",
                "{\"fastestFee\":7,\"halfHourFee\":6,\"hourFee\":5,\"economyFee\":4,\"minimumFee\":3}",
            ),
        ])
        .await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("recommended fee fallback should succeed");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(7));
        assert!(client.estimate_smart_fee_targets().is_empty());
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_when_precise_fee_is_invalid() {
        let server = spawn_response_server(vec![
            (
                "200 OK",
                "{\"fastestFee\":0,\"halfHourFee\":6,\"hourFee\":5,\"economyFee\":4,\"minimumFee\":3}",
            ),
            (
                "200 OK",
                "{\"fastestFee\":7,\"halfHourFee\":6,\"hourFee\":5,\"economyFee\":4,\"minimumFee\":3}",
            ),
        ])
        .await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("recommended fee fallback should succeed");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(7));
        assert!(client.estimate_smart_fee_targets().is_empty());
    }

    #[tokio::test]
    async fn test_mempool_explorer_rejects_oversized_response() {
        let server = spawn_owned_response_server(vec![(
            "200 OK",
            "x".repeat(super::MAX_EXPLORER_RESPONSE_BYTES + 1),
        )])
        .await;
        let explorer = MempoolExplorerClient::new(&server).expect("URL should parse");

        let err = explorer
            .fetch_fee_estimates("api/v1/fees/precise")
            .await
            .expect_err("oversized response should be rejected");

        assert!(matches!(
            err,
            FeeRateError::ExplorerResponseTooLarge {
                endpoint: "api/v1/fees/precise",
                max_bytes: super::MAX_EXPLORER_RESPONSE_BYTES,
            }
        ));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_to_smart_fee_on_invalid_json() {
        let server = spawn_single_response_server("200 OK", "not-json").await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("smart fee fallback should succeed");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(3));
        assert_eq!(client.estimate_smart_fee_targets(), vec![1]);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_to_smart_fee_on_http_error() {
        let server = spawn_single_response_server("500 Internal Server Error", "").await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("smart fee fallback should succeed");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(3));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_gives_bitcoind_a_fresh_budget_after_explorer_timeout() {
        let server = spawn_stalled_server().await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);
        let timeouts =
            FeeRateResolutionTimeouts::new(Duration::from_millis(10), Duration::from_secs(1));

        let fee_rate = resolve_fee_rate(&client, &config, timeouts)
            .await
            .expect("smart fee fallback should get its own timeout budget");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(3));
        assert_eq!(client.estimate_smart_fee_targets(), vec![1]);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_errors_when_mempool_base_url_is_invalid() {
        const SECRET: &str = "malformed-secret";

        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(
            MempoolExplorerFeePolicy::Fastest,
            format!("https://user:{SECRET}@[::1"),
        );

        let err = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect_err("invalid mempool_base_url should error");

        assert!(err.source().is_some());
        assert!(!err.to_string().contains(SECRET));
        assert!(!format!("{err:?}").contains(SECRET));
        assert!(matches!(
            err,
            FeeRateError::InvalidExplorerConfiguration { .. }
        ));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_smart_uses_reader_estimate() {
        let client = TestBitcoinClient::new(1);
        let config = bitcoind_fee_config(1);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("smart fee lookup should succeed");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(3));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_preserves_bitcoin_rpc_error() {
        let source = ClientError::Connection("connection refused".to_string());
        let client = TestBitcoinClient::new(1).with_estimate_smart_fee_error(source.clone());
        let config = bitcoind_fee_config(6);

        let err = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect_err("Bitcoin RPC failure should be returned");

        assert!(matches!(
            err,
            FeeRateError::BitcoinRpc {
                conf_target: 6,
                source: error_source,
            } if error_source == source
        ));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_reports_bitcoin_rpc_timeout() {
        let client =
            TestBitcoinClient::new(1).with_estimate_smart_fee_delay(Duration::from_secs(1));
        let config = bitcoind_fee_config(6);
        let timeouts =
            FeeRateResolutionTimeouts::new(Duration::from_secs(1), Duration::from_millis(10));

        let err = resolve_fee_rate(&client, &config, timeouts)
            .await
            .expect_err("Bitcoin RPC timeout should be returned");

        assert!(matches!(
            err,
            FeeRateError::BitcoinRpcTimeout {
                conf_target: 6,
                timeout,
            } if timeout == Duration::from_millis(10)
        ));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_preserves_both_timeout_failures() {
        let server = spawn_stalled_server().await;
        let client =
            TestBitcoinClient::new(1).with_estimate_smart_fee_delay(Duration::from_secs(1));
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);
        let timeout = Duration::from_millis(10);

        let err = resolve_fee_rate(
            &client,
            &config,
            FeeRateResolutionTimeouts::new(timeout, timeout),
        )
        .await
        .expect_err("both timeout failures should be returned");

        assert!(err.is_timeout());
        assert!(matches!(
            err,
            FeeRateError::Fallback {
                explorer,
                bitcoin_rpc,
            } if matches!(*explorer, FeeRateError::ExplorerTimeout { timeout: value } if value == timeout)
                && matches!(*bitcoin_rpc, FeeRateError::BitcoinRpcTimeout { conf_target: 1, timeout: value } if value == timeout)
        ));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_reports_unavailable_smart_fee_estimate() {
        let errors = Some(vec!["Insufficient data or no feerate found".to_string()]);
        let client = TestBitcoinClient::new(1).with_unavailable_smart_fee_estimate(errors.clone());
        let config = bitcoind_fee_config(6);

        let err = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect_err("unavailable estimate should be returned");

        assert!(matches!(
            err,
            FeeRateError::SmartFeeUnavailable {
                conf_target: 6,
                errors: estimate_errors,
            } if estimate_errors == errors
        ));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_to_smart_fee_on_invalid_explorer_fee_rate() {
        let invalid_response =
            "{\"fastestFee\":0,\"halfHourFee\":6,\"hourFee\":5,\"economyFee\":4,\"minimumFee\":3}";
        let server = spawn_response_server(vec![
            ("200 OK", invalid_response),
            ("200 OK", invalid_response),
        ])
        .await;
        let client = TestBitcoinClient::new(1);
        let config = mempool_fee_config(MempoolExplorerFeePolicy::Fastest, server);

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("smart fee fallback should succeed");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(3));
        assert_eq!(client.estimate_smart_fee_targets(), vec![1]);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_fixed_policy_is_unscaled() {
        let client = TestBitcoinClient::new(1);
        let config = L1FeePolicyConfig::new(FeePolicy::Fixed {
            fee_rate: FeeRate::from_sat_per_vb_u32(9),
        });

        let fee_rate = resolve_fee_rate(&client, &config, resolution_timeouts())
            .await
            .expect("fixed fee policy should resolve");

        assert_eq!(fee_rate, FeeRate::from_sat_per_vb_u32(9));
    }
}
