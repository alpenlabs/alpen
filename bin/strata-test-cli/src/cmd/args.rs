use argh::FromArgs;

use super::{
    convert_to_xonly_pk::ConvertToXonlyPkArgs, create_deposit_tx::CreateDepositTxArgs,
    create_withdrawal_fulfillment::CreateWithdrawalFulfillmentArgs,
    extract_p2tr_pubkey::ExtractP2trPubkeyArgs, get_address::GetAddressArgs,
    musig_aggregate_pks::MusigAggregatePksArgs, sign_schnorr_sig::SignSchnorrSigArgs,
    xonlypk_to_descriptor::XonlypkToDescriptorArgs,
};

/// CLI utilities for Strata functional tests
#[derive(FromArgs, PartialEq, Debug)]
pub struct TopLevel {
    #[argh(subcommand)]
    pub cmd: Commands,
}

/// Available subcommands for the CLI.
///
/// Each variant represents a distinct operation for testing Strata bridge functionality.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub enum Commands {
    /// Create a deposit transaction (DT) from a deposit request transaction (DRT)
    CreateDepositTx(CreateDepositTxArgs),

    /// Create a withdrawal fulfillment transaction (WFT) for bridge withdrawals
    CreateWithdrawalFulfillment(CreateWithdrawalFulfillmentArgs),

    /// Get a taproot address at a specific derivation index
    GetAddress(GetAddressArgs),

    /// Aggregate multiple public keys using MuSig2 protocol
    MusigAggregatePks(MusigAggregatePksArgs),

    /// Extract X-only public key from a taproot address
    ExtractP2trPubkey(ExtractP2trPubkeyArgs),

    /// Convert a public key to X-only format by stripping parity byte
    ConvertToXonlyPk(ConvertToXonlyPkArgs),

    /// Sign a message hash using Schnorr signature scheme
    SignSchnorrSig(SignSchnorrSigArgs),

    /// Convert an X-only public key to a BOSD descriptor
    XonlypkToDescriptor(XonlypkToDescriptorArgs),
}
