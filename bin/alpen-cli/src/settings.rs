use std::{
    fs::{create_dir_all, File},
    io,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, LazyLock},
};

use alloy::primitives::Address as AlpenAddress;
use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, Client};
use bdk_wallet::bitcoin::{Network, XOnlyPublicKey};
use config::Config;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use shrex::Hex;
use terrors::OneOf;

use crate::{
    constants::{BRIDGE_ALPEN_ADDRESS, DEFAULT_NETWORK},
    signet::{backend::SignetBackend, EsploraClient},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsFromFile {
    pub esplora: Option<String>,
    pub bitcoind_rpc_user: Option<String>,
    pub bitcoind_rpc_pw: Option<String>,
    pub bitcoind_rpc_cookie: Option<PathBuf>,
    pub bitcoind_rpc_endpoint: Option<String>,
    pub alpen_endpoint: String,
    pub faucet_endpoint: String,
    pub mempool_endpoint: Option<String>,
    pub blockscout_endpoint: Option<String>,
    pub bridge_pubkey: Hex<[u8; 32]>,
    pub magic_bytes: String,
    pub network: Option<Network>,
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
    pub magic_bytes: String,
    pub linux_seed_file: PathBuf,
    pub network: Network,
    pub config_file: PathBuf,
    pub signet_backend: Arc<dyn SignetBackend>,
}

pub static PROJ_DIRS: LazyLock<ProjectDirs> = LazyLock::new(|| {
    ProjectDirs::from("io", "alpenlabs", "alpen").expect("project dir should be available")
});

pub static CONFIG_FILE: LazyLock<PathBuf> =
    LazyLock::new(|| match std::env::var("CLI_CONFIG").ok() {
        Some(path) => PathBuf::from_str(&path).expect("valid config path"),
        None => PROJ_DIRS.config_dir().to_owned().join("config.toml"),
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
            bridge_alpen_address: AlpenAddress::from_str(BRIDGE_ALPEN_ADDRESS)
                .expect("valid Alpen address"),
            magic_bytes: from_file.magic_bytes,
            linux_seed_file,
            network: from_file.network.unwrap_or(DEFAULT_NETWORK),
            config_file: CONFIG_FILE.clone(),
            signet_backend: sync_backend,
        })
    }
}

#[cfg(test)]
mod tests {
    use toml;

    use super::*;
    use crate::constants::MAGIC_BYTES_LEN;

    #[test]
    fn test_magic_bytes_length() {
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
            magic_bytes = "alpn"
            network = "signet"
        "#;

        let parsed: SettingsFromFile =
            toml::from_str(config).expect("failed to parse SettingsFromFile from TOML");
        assert!(parsed.magic_bytes.as_bytes().len() == MAGIC_BYTES_LEN);
    }

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
            magic_bytes = "alpn"
            network = "signet"
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
        assert_eq!(parsed.magic_bytes, reparsed.magic_bytes);
        assert_eq!(parsed.network, reparsed.network);
        assert_eq!(parsed.bridge_pubkey.0, reparsed.bridge_pubkey.0);
    }
}
