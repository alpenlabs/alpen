//! Module for resolving fee rates for transactions, supporting multiple fee policies including
//! Bitcoin Core's `estimatesmartfee` and mempool.space's recommended fees endpoint.

use anyhow::{anyhow, Context};
use bitcoind_async_client::traits::Reader;
use reqwest::Url;
use serde::Deserialize;
use strata_config::btcio::{FeePolicy, WriterConfig};
use tracing::warn;

/// Represents the response from the mempool.space recommended fees endpoint.
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

/// Resolves the fee rate to use for a transaction based on the provided configuration.
pub(crate) async fn resolve_fee_rate<R: Reader>(
    client: &R,
    config: &WriterConfig,
) -> anyhow::Result<u64> {
    match &config.fee_policy {
        FeePolicy::BitcoinD => client
            .estimate_smart_fee(1)
            .await
            .context("failed to estimate smart fee"),
        FeePolicy::Mempool => resolve_mempool_fee_rate(client, config).await,
        FeePolicy::Fixed(value) => Ok(*value),
    }
}

/// Resolves the fee rate using the mempool.space recommended fees endpoint, falling back to
/// Bitcoin Core's `estimatesmartfee` on failure.
async fn resolve_mempool_fee_rate<R: Reader>(
    client: &R,
    config: &WriterConfig,
) -> anyhow::Result<u64> {
    let base_url = config
        .mempool_base_url
        .as_deref()
        .ok_or_else(|| anyhow!("mempool_base_url must be set when fee_policy = \"mempool\""))?;
    let url = mempool_recommended_fees_url(base_url)?;

    match fetch_mempool_recommended_fees(url).await {
        Ok(fees) => Ok(fees.fastest_fee),
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

fn mempool_recommended_fees_url(base_url: &str) -> anyhow::Result<Url> {
    let mut url =
        Url::parse(base_url).with_context(|| format!("invalid mempool_base_url: {base_url}"))?;

    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }

    url.join("api/v1/fees/recommended")
        .with_context(|| format!("invalid recommended-fees URL for base: {base_url}"))
}

async fn fetch_mempool_recommended_fees(url: Url) -> anyhow::Result<MempoolRecommendedFees> {
    reqwest::get(url)
        .await
        .context("failed to call mempool recommended fees endpoint")?
        .error_for_status()
        .context("mempool recommended fees endpoint returned an error status")?
        .json::<MempoolRecommendedFees>()
        .await
        .context("failed to decode mempool recommended fees response")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_config::btcio::{FeePolicy, WriterConfig};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::{mempool_recommended_fees_url, resolve_fee_rate, MempoolRecommendedFees};
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
    fn test_mempool_recommended_fees_url_handles_trailing_slash() {
        let without_slash =
            mempool_recommended_fees_url("https://mempool.space/signet").expect("url should parse");
        let with_slash = mempool_recommended_fees_url("https://mempool.space/signet/")
            .expect("url should parse");

        assert_eq!(
            without_slash.as_str(),
            "https://mempool.space/signet/api/v1/fees/recommended"
        );
        assert_eq!(without_slash, with_slash);
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
            fee_policy: FeePolicy::Mempool,
            mempool_base_url: Some(server),
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(&client, &config)
            .await
            .expect("mempool fee lookup should succeed");

        assert_eq!(fee_rate, 7);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_to_smart_fee_on_invalid_json() {
        let server = spawn_single_response_server("200 OK", "not-json").await;
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::Mempool,
            mempool_base_url: Some(server),
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(&client, &config)
            .await
            .expect("smart fee fallback should succeed");

        assert_eq!(fee_rate, 3);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_falls_back_to_smart_fee_on_http_error() {
        let server = spawn_single_response_server("500 Internal Server Error", "").await;
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::Mempool,
            mempool_base_url: Some(server),
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(&client, &config)
            .await
            .expect("smart fee fallback should succeed");

        assert_eq!(fee_rate, 3);
    }

    #[tokio::test]
    async fn test_resolve_fee_rate_errors_when_mempool_base_url_is_missing() {
        let client = TestBitcoinClient::new(1);
        let config = WriterConfig {
            fee_policy: FeePolicy::Mempool,
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
            fee_policy: FeePolicy::Mempool,
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
            fee_policy: FeePolicy::BitcoinD,
            ..WriterConfig::default()
        };

        let fee_rate = resolve_fee_rate(client.as_ref(), &config)
            .await
            .expect("smart fee lookup should succeed");

        assert_eq!(fee_rate, 3);
    }
}
