use std::{
    env::var,
    fs::{create_dir_all, read_to_string, File},
    io,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, LazyLock},
};

use alloy::primitives::Address as AlpenAddress;
use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, Client};
use bdk_wallet::bitcoin::{Amount, XOnlyPublicKey};
use config::Config;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use shrex::Hex;
use strata_params::RollupParams;
use terrors::OneOf;

#[cfg(feature = "test-mode")]
use crate::{constants::SEED_LEN, seed::Seed};
use crate::{
    constants::{DEFAULT_BRIDGE_ALPEN_ADDRESS, DEFAULT_BRIDGE_FEE, DEFAULT_FINALITY_DEPTH},
    signet::{backend::SignetBackend, EsploraClient},
};

/// Settings deserialized from the config file.
#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsFromFile {
    /// Esplora server endpoint.
    pub esplora: Option<String>,
    /// Bitcoind RPC username.
    pub bitcoind_rpc_user: Option<String>,
    /// Bitcoind RPC password.
    pub bitcoind_rpc_pw: Option<String>,
    /// Path to the Bitcoind RPC cookie file.
    pub bitcoind_rpc_cookie: Option<PathBuf>,
    /// Bitcoind RPC endpoint.
    pub bitcoind_rpc_endpoint: Option<String>,
    /// Alpen network RPC endpoint.
    pub alpen_endpoint: String,
    /// Faucet service endpoint.
    pub faucet_endpoint: String,
    /// Mempool explorer endpoint.
    pub mempool_endpoint: Option<String>,
    /// Blockscout explorer endpoint.
    pub blockscout_endpoint: Option<String>,
    /// The aggregated Musig2 public key for the bridge.
    pub bridge_pubkey: Hex<[u8; 32]>,
    /// The address of the bridge precompile in alpen evm in hex.
    pub bridge_alpen_address: Option<String>,
    /// Fee to cover mining costs for the bridge to process deposits, in satoshis.
    pub bridge_fee_sats: Option<u64>,
    /// The number of confirmations to consider a Bitcoin transaction final.
    pub finality_depth: Option<u32>,
    /// Path to the rollup params JSON file.
    pub rollup_params_path: Option<PathBuf>,
    /// Seed that can be passed directly for functional test.
    #[cfg(feature = "test-mode")]
    pub seed: Hex<[u8; SEED_LEN]>,
}

/// Settings struct filled with either config values or
/// opinionated defaults
#[derive(Debug)]
pub struct Settings {
    pub esplora: Option<String>,
    pub alpen_endpoint: String,
    pub data_dir: PathBuf,
    pub faucet_endpoint: String,
    pub bridge_musig2_pubkey: XOnlyPublicKey,
    pub descriptor_db: PathBuf,
    pub mempool_space_endpoint: Option<String>,
    pub blockscout_endpoint: Option<String>,
    pub bridge_alpen_address: AlpenAddress,
    pub linux_seed_file: PathBuf,
    pub config_file: PathBuf,
    pub signet_backend: Arc<dyn SignetBackend>,
    pub bridge_fee: Amount,
    pub finality_depth: u32,
    pub params: RollupParams,
    #[cfg(feature = "test-mode")]
    pub seed: Seed,
}

pub static PROJ_DIRS: LazyLock<ProjectDirs> = LazyLock::new(|| match var("PROJ_DIRS").ok() {
    Some(path) => ProjectDirs::from_path(path.into()).expect("valid project path"),
    None => ProjectDirs::from("io", "alpenlabs", "alpen").expect("project dir should be available"),
});

pub static CONFIG_FILE: LazyLock<PathBuf> = LazyLock::new(|| match var("CLI_CONFIG").ok() {
    Some(path) => PathBuf::from_str(&path).expect("valid config path"),
    None => PROJ_DIRS.config_dir().to_owned().join("config.toml"),
});

pub static ROLLUP_PARAMS_FILE: LazyLock<PathBuf> =
    LazyLock::new(|| match var("STRATA_NETWORK_PARAMS").ok() {
        Some(path) => PathBuf::from_str(&path).expect("valid rollup params path"),
        None => PROJ_DIRS.config_dir().to_owned().join("rollup_params.json"),
    });

impl Settings {
    pub fn load() -> Result<Self, OneOf<(io::Error, config::ConfigError)>> {
        let proj_dirs = &PROJ_DIRS;
        let config_file = CONFIG_FILE.as_path();
        let descriptor_file = proj_dirs.data_dir().to_owned().join("descriptors");
        let linux_seed_file = proj_dirs.data_dir().to_owned().join("seed");

        create_dir_all(proj_dirs.config_dir()).map_err(OneOf::new)?;
        create_dir_all(proj_dirs.data_dir()).map_err(OneOf::new)?;

        // create config file if not exists
        let _ = File::create_new(config_file);
        let from_file: SettingsFromFile = Config::builder()
            .add_source(config::File::from(config_file))
            .build()
            .map_err(OneOf::new)?
            .try_deserialize::<SettingsFromFile>()
            .map_err(OneOf::new)?;

        let sync_backend: Arc<dyn SignetBackend> = match (
            from_file.esplora.clone(),
            from_file.bitcoind_rpc_user,
            from_file.bitcoind_rpc_pw,
            from_file.bitcoind_rpc_cookie,
            from_file.bitcoind_rpc_endpoint,
        ) {
            (Some(url), None, None, None, None) => {
                Arc::new(EsploraClient::new(&url).expect("valid esplora url"))
            }
            (None, Some(user), Some(pw), None, Some(url)) => Arc::new(Arc::new(
                Client::new(&url, Auth::UserPass(user, pw)).expect("valid bitcoin core client"),
            )),
            (None, None, None, Some(cookie_file), Some(url)) => Arc::new(Arc::new(
                Client::new(&url, Auth::CookieFile(cookie_file))
                    .expect("valid bitcoin core client"),
            )),
            _ => panic!("invalid config for signet - configure for esplora or bitcoind"),
        };

        // Load rollup params from config, environment variable, or default location
        let params_path = from_file
            .rollup_params_path
            .unwrap_or_else(|| ROLLUP_PARAMS_FILE.clone());

        let params_json = read_to_string(&params_path).map_err(OneOf::new)?;
        let params: RollupParams = serde_json::from_str(&params_json).map_err(|e| {
            OneOf::new(config::ConfigError::Message(format!(
                "Failed to parse rollup params: {}",
                e
            )))
        })?;

        Ok(Settings {
            esplora: from_file.esplora,
            alpen_endpoint: from_file.alpen_endpoint,
            data_dir: proj_dirs.data_dir().to_owned(),
            faucet_endpoint: from_file.faucet_endpoint,
            bridge_musig2_pubkey: XOnlyPublicKey::from_slice(&from_file.bridge_pubkey.0)
                .expect("valid length"),
            descriptor_db: descriptor_file,
            mempool_space_endpoint: from_file.mempool_endpoint,
            blockscout_endpoint: from_file.blockscout_endpoint,
            bridge_alpen_address: AlpenAddress::from_str(
                from_file
                    .bridge_alpen_address
                    .as_deref()
                    .unwrap_or(DEFAULT_BRIDGE_ALPEN_ADDRESS),
            )
            .expect("valid Alpen address"),
            linux_seed_file,
            config_file: CONFIG_FILE.clone(),
            signet_backend: sync_backend,
            bridge_fee: from_file
                .bridge_fee_sats
                .map(Amount::from_sat)
                .unwrap_or(DEFAULT_BRIDGE_FEE),
            finality_depth: from_file.finality_depth.unwrap_or(DEFAULT_FINALITY_DEPTH),
            params,
            #[cfg(feature = "test-mode")]
            seed: Seed::from_file(from_file.seed),
        })
    }
}

#[cfg(test)]
mod tests {
    use toml;

    use super::*;

    #[test]
    fn test_settings_from_file_serde_roundtrip() {
        let config = r#"
            esplora = "https://esplora.testnet.alpenlabs.io"
            bitcoind_rpc_user = "user"
            bitcoind_rpc_pw = "pass"
            bitcoind_rpc_endpoint = "http://127.0.0.1:38332"
            alpen_endpoint = "https://rpc.testnet.alpenlabs.io"
            faucet_endpoint = "https://faucet-api.testnet.alpenlabs.io"
            mempool_endpoint = "https://bitcoin.testnet.alpenlabs.io"
            blockscout_endpoint = "https://explorer.testnet.alpenlabs.io"
            bridge_pubkey = "1d3e9c0417ba7d3551df5a1cc1dbe227aa4ce89161762454d92bfc2b1d5886f7"
        "#;

        // Deserialize from TOML string
        let parsed: SettingsFromFile =
            toml::from_str(config).expect("failed to parse SettingsFromFile from TOML");

        // Serialize back to TOML string
        let serialized =
            toml::to_string(&parsed).expect("failed to serialize SettingsFromFile to TOML");

        // Deserialize again
        let reparsed: SettingsFromFile =
            toml::from_str(&serialized).expect("failed to deserialize serialized SettingsFromFile");

        // Assert important fields survived round-trip
        assert_eq!(parsed.esplora, reparsed.esplora);
        assert_eq!(parsed.alpen_endpoint, reparsed.alpen_endpoint);
        assert_eq!(parsed.faucet_endpoint, reparsed.faucet_endpoint);
        assert_eq!(parsed.bridge_pubkey.0, reparsed.bridge_pubkey.0);
    }
}
