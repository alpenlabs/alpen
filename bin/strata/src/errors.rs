//! Error types for initialization and configuration.

use std::io;

use format_serde_error::SerdeError;
use strata_params::ParamsError;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum InitError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("unparsable params file: {0}")]
    UnparsableParamsFile(#[from] SerdeError),

    #[error("config: {0}")]
    MalformedConfig(#[from] ConfigError),

    #[error("params: {0}")]
    MalformedParams(#[from] ParamsError),

    #[error("missing rollup params path in arguments")]
    MissingRollupParams,

    #[error("invalid datadir path: {0:?}")]
    InvalidDatadirPath(std::path::PathBuf),

    #[error("failed to build tokio runtime: {0}")]
    RuntimeBuild(#[source] io::Error),

    #[error("failed to create node storage: {0}")]
    StorageCreation(String),

    #[error("missing sync endpoint (required for non-sequencer nodes)")]
    MissingSyncEndpoint,

    #[error("failed to parse TOML configuration: {0}")]
    TomlParse(#[source] toml::de::Error),

    #[error("failed to create bitcoin RPC client: {0}")]
    BitcoinClientCreation(String),

    #[error("missing initial client state in database")]
    MissingInitialState,
}

#[derive(Debug, Error)]
pub(crate) enum ConfigError {
    #[error("missing key '{key}' in path '{path}'")]
    MissingKey { key: String, path: String },

    #[error("attempt to traverse into non-table at key '{key}' in path '{path}'")]
    TraverseNonTableAt { key: String, path: String },

    #[error("invalid override string '{override_str}' (expected format: 'key.path=value')")]
    InvalidOverride { override_str: String },
}
