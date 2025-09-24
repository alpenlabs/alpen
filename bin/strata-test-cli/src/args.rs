//! Command-line argument definitions for strata-test-cli
//!
//! This module defines all CLI argument structures using argh.

use argh::FromArgs;

#[derive(FromArgs)]
/// CLI utilities for Strata functional tests
pub struct Args {
    #[argh(subcommand)]
    pub command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
pub enum Command {
    CreateDepositTx(CreateDepositTxArgs),
    CreateWithdrawalFulfillment(CreateWithdrawalFulfillmentArgs),
    GetAddress(GetAddressArgs),
    MusigAggregatePks(MusigAggregatePksArgs),
    ExtractP2trPubkey(ExtractP2trPubkeyArgs),
    ConvertToXonlyPk(ConvertToXonlyPkArgs),
    SignSchnorrSig(SignSchnorrSigArgs),
    XonlypkToDescriptor(XonlypkToDescriptorArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "create-deposit-tx")]
/// Create a deposit transaction from DRT
pub struct CreateDepositTxArgs {
    #[argh(option)]
    /// raw DRT transaction in hex format
    pub drt_tx: String,

    #[argh(option)]
    /// operator private keys in JSON array format (each key is 78 bytes hex)
    pub operator_keys: String,

    #[argh(option)]
    /// deposit transaction index
    pub index: u32,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "create-withdrawal-fulfillment")]
/// Create a withdrawal fulfillment transaction
pub struct CreateWithdrawalFulfillmentArgs {
    #[argh(option)]
    /// destination Bitcoin address (BOSD format)
    pub destination: String,

    #[argh(option)]
    /// amount in satoshis
    pub amount: u64,

    #[argh(option)]
    /// operator index
    pub operator_idx: u32,

    #[argh(option)]
    /// deposit index
    pub deposit_idx: u32,

    #[argh(option)]
    /// deposit transaction ID (hex)
    pub deposit_txid: String,

    #[argh(option)]
    /// bitcoin RPC URL
    pub btc_url: String,

    #[argh(option)]
    /// bitcoin RPC username
    pub btc_user: String,

    #[argh(option)]
    /// bitcoin RPC password
    pub btc_password: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "get-address")]
/// Get a taproot address at a specific index
pub struct GetAddressArgs {
    #[argh(option)]
    /// address index
    pub index: u32,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "musig-aggregate-pks")]
/// Aggregate public keys using MuSig2
pub struct MusigAggregatePksArgs {
    #[argh(option)]
    /// public keys in JSON array format (hex strings)
    pub pubkeys: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "extract-p2tr-pubkey")]
/// Extract P2TR public key from a taproot address
pub struct ExtractP2trPubkeyArgs {
    #[argh(option)]
    /// taproot address
    pub address: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "convert-to-xonly-pk")]
/// Convert a public key to X-only format
pub struct ConvertToXonlyPkArgs {
    #[argh(option)]
    /// public key in hex format
    pub pubkey: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "sign-schnorr-sig")]
/// Sign a message using Schnorr signature
pub struct SignSchnorrSigArgs {
    #[argh(option)]
    /// message hash in hex format
    pub message: String,

    #[argh(option)]
    /// secret key in hex format
    pub secret_key: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "xonlypk-to-descriptor")]
/// Convert X-only public key to BOSD descriptor
pub struct XonlypkToDescriptorArgs {
    #[argh(option)]
    /// x-only public key in hex format
    pub xonly_pubkey: String,
}
