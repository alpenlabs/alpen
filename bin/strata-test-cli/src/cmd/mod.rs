use argh::FromArgs;
use convert_to_xonly_pk::ConvertToXonlyPkArgs;
use create_deposit_tx::CreateDepositTxArgs;
use create_withdrawal_fulfillment::CreateWithdrawalFulfillmentArgs;
use extract_p2tr_pubkey::ExtractP2trPubkeyArgs;
use get_address::GetAddressArgs;
use musig_aggregate_pks::MusigAggregatePksArgs;
use sign_schnorr_sig::SignSchnorrSigArgs;
use xonlypk_to_descriptor::XonlypkToDescriptorArgs;

pub mod convert_to_xonly_pk;
pub mod create_deposit_tx;
pub mod create_withdrawal_fulfillment;
pub mod extract_p2tr_pubkey;
pub mod get_address;
pub mod musig_aggregate_pks;
pub mod sign_schnorr_sig;
pub mod xonlypk_to_descriptor;

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
    CreateDepositTx(CreateDepositTxArgs),
    CreateWithdrawalFulfillment(CreateWithdrawalFulfillmentArgs),
    GetAddress(GetAddressArgs),
    MusigAggregatePks(MusigAggregatePksArgs),
    ExtractP2trPubkey(ExtractP2trPubkeyArgs),
    ConvertToXonlyPk(ConvertToXonlyPkArgs),
    SignSchnorrSig(SignSchnorrSigArgs),
    XonlypkToDescriptor(XonlypkToDescriptorArgs),
}
