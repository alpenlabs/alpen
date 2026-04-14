//! Module for resolving fee rates for transactions, supporting multiple fee policies including
//! Bitcoin Core's `estimatesmartfee` and mempool.space's recommended fees endpoint.

use anyhow::{anyhow, Context};
use bitcoind_async_client::traits::Reader;
use reqwest::Url;
use serde::Deserialize;
use strata_config::btcio::{FeePolicy, MempoolExplorerFeePolicy, WriterConfig};
use tracing::warn;

/// Represents the response from the mempool explorer recommended fees endpoint.
// TODO(STR-3038): once we update Alpen's mempool explorers we can use `api/v1/fees/precise`
//                 for more granular sub-1 sat/vB fee rates if desired.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct MempoolRecommendedFees {
    #[serde(rename = "fastestFee")]
    fastest_fee: u64,
    #[serde(rename = "halfHourFee")]
    half_hour_fee: u64,
    #[serde(rename = "hourFee")]
    hour_fee: u64,
    #[serde(rename = "economyFee")]
    economy_fee: u64,
    #[serde(rename = "minimumFee")]
    minimum_fee: u64,
}

impl MempoolRecommendedFees {
    /// Selects the fee rate according to the given policy.
    fn select(self, policy: MempoolExplorerFeePolicy) -> u64 {
        match policy {
            MempoolExplorerFeePolicy::Fastest => self.fastest_fee,
            MempoolExplorerFeePolicy::HalfHour => self.half_hour_fee,
            MempoolExplorerFeePolicy::Hour => self.hour_fee,
            MempoolExplorerFeePolicy::Economy => self.economy_fee,
            MempoolExplorerFeePolicy::Minimum => self.minimum_fee,
        }
    }
}

/// HTTP client for querying a mempool explorer's fee estimation API.
struct MempoolExplorerClient {
    base_url: Url,
    http: reqwest::Client,
}

impl MempoolExplorerClient {
    /// Creates a new client from a base URL string (e.g. `https://mempool.space/signet`).
    fn new(base_url: &str) -> anyhow::Result<Self> {
        let mut url = Url::parse(base_url)
            .with_context(|| format!("invalid mempool_base_url: {base_url}"))?;

        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }

        Ok(Self {
            base_url: url,
            http: reqwest::Client::new(),
        })
    }

    /// Fetches the recommended fees from the mempool explorer.
    async fn fetch_recommended_fees(&self) -> anyhow::Result<MempoolRecommendedFees> {
        let url = self
            .base_url
            .join("api/v1/fees/recommended")
            .with_context(|| format!("invalid recommended-fees URL for base: {}", self.base_url))?;

        self.http
            .get(url)
            .send()
            .await
            .context("failed to call mempool recommended fees endpoint")?
            .error_for_status()
            .context("mempool recommended fees endpoint returned an error status")?
            .json::<MempoolRecommendedFees>()
            .await
            .context("failed to decode mempool recommended fees response")
    }
}

/// Resolves the fee rate to use for a transaction based on the provided configuration.
pub(crate) async fn resolve_fee_rate<R: Reader>(
    client: &R,
    config: &WriterConfig,
) -> anyhow::Result<u64> {
    let fee_rate = match &config.fee_policy {
        FeePolicy::BitcoinD { conf_target } => client
            .estimate_smart_fee(*conf_target)
            .await
            .context("failed to estimate smart fee"),
        FeePolicy::MempoolExplorer { policy } => {
            resolve_mempool_fee_rate(client, config, *policy).await
        }
        FeePolicy::Fixed { fee_rate } => Ok(*fee_rate),
    }?;

    // NOTE(STR-2545, STR-2433): fee estimation is currently being doubled since we don't fully
    //                           support fee bumping ostensive mechanisms for making sure
    //                           transactions confirm in a timely manner. This is a temporary
    //                           measure until we implement more robust fee bumping strategies,
    //                           at which point we can remove the doubling and rely on the fee
    //                           policies to provide accurate fee rates.
    Ok(fee_rate * 2)
}

/// Resolves the fee rate using the mempool explorer recommended fees endpoint, falling back to
/// Bitcoin Core's `estimatesmartfee` on failure.
async fn resolve_mempool_fee_rate<R: Reader>(
    client: &R,
    config: &WriterConfig,
    mempool_fee_policy: MempoolExplorerFeePolicy,
) -> anyhow::Result<u64> {
    let base_url = config
        .mempool_base_url
        .as_deref()
        .ok_or_else(|| anyhow!("mempool_base_url must be set when fee_policy = \"mempool\""))?;

    let explorer = MempoolExplorerClient::new(base_url)?;

    match explorer.fetch_recommended_fees().await {
        Ok(fees) => Ok(fees.select(mempool_fee_policy)),
        Err(err) => {
            warn!(
                %base_url,
                %err,
                "mempool fee lookup failed, falling back to bitcoind's estimatesmartfee"
            );
            client
                .estimate_smart_fee(1)
                .await
                .context("failed to estimate smart fee after mempool fallback")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_config::btcio::{FeePolicy, MempoolExplorerFeePolicy, WriterConfig};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::{resolve_fee_rate, MempoolExplorerClient, MempoolRecommendedFees};
    use crate::test_utils::TestBitcoinClient;

    async fn spawn_single_response_server(status_line: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");

        tokio::spawn(async move {
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
                fastest_fee: 1,
                half_hour_fee: 2,
                hour_fee: 3,
                economy_fee: 4,
                minimum_fee: 5,
            }
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
    async fn test_resolve_fee_rate_uses_mempool_fastest_fee() {
        let server = spawn_single_response_server(
            "200 OK",
            "{\"fastestFee\":7,\"halfHourFee\":6,\"hourFee\":5,\"economyFee\":4,\"minimumFee\":3}",
        )
        .await;
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::MempoolExplorer {
                policy: MempoolExplorerFeePolicy::Fastest,
            },
            mempool_base_url: Some(server),
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(&client, &config)
            .await
            .expect("mempool fee lookup should succeed");

        // NOTE: double the fees here
        assert_eq!(fee_rate, 7 * 2);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_uses_selected_mempool_policy() {
        let server = spawn_single_response_server(
            "200 OK",
            "{\"fastestFee\":7,\"halfHourFee\":6,\"hourFee\":5,\"economyFee\":4,\"minimumFee\":3}",
        )
        .await;
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::MempoolExplorer {
                policy: MempoolExplorerFeePolicy::Economy,
            },
            mempool_base_url: Some(server),
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(&client, &config)
            .await
            .expect("mempool fee lookup should succeed");

        // NOTE: double the fees here
        assert_eq!(fee_rate, 4 * 2);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_to_smart_fee_on_invalid_json() {
        let server = spawn_single_response_server("200 OK", "not-json").await;
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::MempoolExplorer {
                policy: MempoolExplorerFeePolicy::Fastest,
            },
            mempool_base_url: Some(server),
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(&client, &config)
            .await
            .expect("smart fee fallback should succeed");

        // NOTE: double the fees here
        assert_eq!(fee_rate, 3 * 2);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_to_smart_fee_on_http_error() {
        let server = spawn_single_response_server("500 Internal Server Error", "").await;
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::MempoolExplorer {
                policy: MempoolExplorerFeePolicy::Fastest,
            },
            mempool_base_url: Some(server),
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(&client, &config)
            .await
            .expect("smart fee fallback should succeed");

        // NOTE: double the fees here
        assert_eq!(fee_rate, 3 * 2);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_errors_when_mempool_base_url_is_missing() {
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::MempoolExplorer {
                policy: MempoolExplorerFeePolicy::Fastest,
            },
            mempool_base_url: None,
            ..WriterConfig::default()
        };

        let err = resolve_fee_rate(&client, &config)
            .await
            .expect_err("missing mempool_base_url should error");

        assert!(err
            .to_string()
            .contains("mempool_base_url must be set when fee_policy = \"mempool\""));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_errors_when_mempool_base_url_is_invalid() {
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::MempoolExplorer {
                policy: MempoolExplorerFeePolicy::Fastest,
            },
            mempool_base_url: Some("not a url".to_string()),
            ..WriterConfig::default()
        };

        let err = resolve_fee_rate(&client, &config)
            .await
            .expect_err("invalid mempool_base_url should error");

        assert!(err.to_string().contains("invalid mempool_base_url"));
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_smart_uses_raw_estimate() {
        let client = Arc::new(TestBitcoinClient::new(1));
        let config = WriterConfig {
            fee_policy: FeePolicy::BitcoinD { conf_target: 1 },
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(client.as_ref(), &config)
            .await
            .expect("smart fee lookup should succeed");

        // NOTE: double the fees here
        assert_eq!(fee_rate, 3 * 2);
    }
}
