//! Chain specification for the reth node.

use std::{fs, path::PathBuf, sync::Arc};

use alloy_genesis::Genesis;
use reth_chainspec::ChainSpec;
use reth_cli::chainspec::ChainSpecParser;

pub const DEFAULT_CHAIN_SPEC: &str = include_str!("res/testnet-chain.json");
pub const DEVNET_CHAIN_SPEC: &str = include_str!("res/devnet-chain.json");
pub const DEV_CHAIN_SPEC: &str = include_str!("res/alpen-dev-chain.json");

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AlpenChainSpecParser;

impl ChainSpecParser for AlpenChainSpecParser {
    type ChainSpec = ChainSpec;

    const SUPPORTED_CHAINS: &'static [&'static str] = &["dev", "devnet", "testnet"];

    fn parse(s: &str) -> eyre::Result<Arc<Self::ChainSpec>> {
        chain_value_parser(s)
    }
}

pub fn chain_value_parser(s: &str) -> eyre::Result<Arc<ChainSpec>, eyre::Error> {
    Ok(match s {
        "testnet" => parse_chain_spec(DEFAULT_CHAIN_SPEC)?,
        "devnet" => parse_chain_spec(DEVNET_CHAIN_SPEC)?,
        "dev" => parse_chain_spec(DEV_CHAIN_SPEC)?,
        _ => {
            // try to read json from path first
            let raw = match fs::read_to_string(PathBuf::from(shellexpand::full(s)?.into_owned())) {
                Ok(raw) => raw,
                Err(io_err) => {
                    // valid json may start with "\n", but must contain "{"
                    if s.contains('{') {
                        s.to_string()
                    } else {
                        return Err(io_err.into()); // assume invalid path
                    }
                }
            };

            // both serialized Genesis and ChainSpec structs supported
            let genesis: Genesis = serde_json::from_str(&raw)?;

            Arc::new(genesis.into())
        }
    })
}

fn parse_chain_spec(chain_json: &str) -> eyre::Result<Arc<ChainSpec>> {
    // both serialized Genesis and ChainSpec structs supported
    let genesis: Genesis = serde_json::from_str(chain_json)?;

    Ok(Arc::new(genesis.into()))
}
