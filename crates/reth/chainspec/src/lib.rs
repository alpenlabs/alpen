//! Chain specification for the reth node.

use std::{fs, path::PathBuf, sync::Arc};

use alloy_genesis::Genesis;
use alloy_primitives::B256;
use reth_chainspec::ChainSpec;
use reth_cli::chainspec::ChainSpecParser;

pub const DEFAULT_CHAIN_SPEC: &str = include_str!("res/testnet-chain.json");
pub const DEVNET_CHAIN_SPEC: &str = include_str!("res/devnet-chain.json");
pub const DEV_CHAIN_SPEC: &str = include_str!("res/alpen-dev-chain.json");
pub const TESTNET3_CHAIN_SPEC: &str = include_str!("res/testnet3-chain.json");

/// Genesis block data that must match the Alpen EE params file.
#[derive(Debug)]
pub struct AlpenEeGenesisBlockInfo {
    blockhash: B256,
    stateroot: B256,
    blocknum: u64,
}

impl AlpenEeGenesisBlockInfo {
    /// Returns the execution genesis block hash.
    pub fn blockhash(&self) -> B256 {
        self.blockhash
    }

    /// Returns the execution genesis state root.
    pub fn stateroot(&self) -> B256 {
        self.stateroot
    }

    /// Returns the execution genesis block number.
    pub fn blocknum(&self) -> u64 {
        self.blocknum
    }
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AlpenChainSpecParser;

impl ChainSpecParser for AlpenChainSpecParser {
    type ChainSpec = ChainSpec;

    const SUPPORTED_CHAINS: &'static [&'static str] = &["dev", "devnet", "testnet", "testnet3"];

    fn parse(s: &str) -> eyre::Result<Arc<Self::ChainSpec>> {
        chain_value_parser(s)
    }
}

pub fn chain_value_parser(s: &str) -> eyre::Result<Arc<ChainSpec>, eyre::Error> {
    Ok(match s {
        "testnet" => parse_chain_spec(DEFAULT_CHAIN_SPEC)?,
        "devnet" => parse_chain_spec(DEVNET_CHAIN_SPEC)?,
        "dev" => parse_chain_spec(DEV_CHAIN_SPEC)?,
        "testnet3" => parse_chain_spec(TESTNET3_CHAIN_SPEC)?,
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

/// Extracts Alpen EE genesis block info from a chain spec.
pub fn ee_genesis_block_info(chain_spec: &ChainSpec) -> AlpenEeGenesisBlockInfo {
    let genesis_header = chain_spec.genesis_header();
    let genesis_stateroot = genesis_header.state_root;
    let genesis_hash = chain_spec.genesis_hash();
    let genesis_blocknum = chain_spec
        .genesis()
        .number
        .expect("genesis block number should be present");

    AlpenEeGenesisBlockInfo {
        blockhash: genesis_hash,
        stateroot: genesis_stateroot,
        blocknum: genesis_blocknum,
    }
}

/// Extracts Alpen EE genesis block info from a serialized genesis JSON value.
pub fn ee_genesis_block_info_from_json(
    chain_json: &str,
) -> serde_json::Result<AlpenEeGenesisBlockInfo> {
    let genesis: Genesis = serde_json::from_str(chain_json)?;
    let chain_spec = ChainSpec::from_genesis(genesis);

    Ok(ee_genesis_block_info(&chain_spec))
}

fn parse_chain_spec(chain_json: &str) -> eyre::Result<Arc<ChainSpec>> {
    // both serialized Genesis and ChainSpec structs supported
    let genesis: Genesis = serde_json::from_str(chain_json)?;

    Ok(Arc::new(genesis.into()))
}
