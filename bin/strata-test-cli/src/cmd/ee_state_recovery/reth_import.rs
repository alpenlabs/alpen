//! Imports a reconstructed state dump through Reth's state initializer.

use std::{ffi::OsString, path::PathBuf};

use alloy_primitives::B256;
use alpen_chainspec::AlpenChainSpecParser;
use alpen_reth_node::AlpenEthereumNode;
use clap::Parser;
use reth_cli_commands::init_state::InitStateCommand;
use tokio::runtime::Builder;

/// Import a reconstructed JSONL state dump into a fresh Reth database.
#[derive(Debug, PartialEq)]
pub(super) struct RethImportConfig {
    /// fresh Reth datadir to initialize
    pub datadir: PathBuf,

    /// chain specification name or path
    pub chain: String,

    /// reconstructed JSONL state dump
    pub state: PathBuf,

    /// initialize at the supplied non-genesis header without historical EVM blocks
    pub without_evm: bool,

    /// rlp-encoded reconstructed anchor header
    pub header: Option<PathBuf>,

    /// optional externally committed hash for the anchor header
    pub header_hash: Option<B256>,
}

pub(super) fn import(args: RethImportConfig) -> eyre::Result<()> {
    let mut command_args = vec![
        OsString::from("ee-state-import"),
        OsString::from("--datadir"),
        args.datadir.into_os_string(),
        OsString::from("--chain"),
        OsString::from(args.chain),
    ];
    if args.without_evm {
        command_args.push(OsString::from("--without-evm"));
    }
    if let Some(header) = args.header {
        command_args.push(OsString::from("--header"));
        command_args.push(header.into_os_string());
    }
    if let Some(header_hash) = args.header_hash {
        command_args.push(OsString::from("--header-hash"));
        command_args.push(OsString::from(header_hash.to_string()));
    }
    command_args.push(args.state.into_os_string());

    let command = InitStateCommand::<AlpenChainSpecParser>::try_parse_from(command_args)?;
    let runtime = Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(command.execute::<AlpenEthereumNode>())
}
