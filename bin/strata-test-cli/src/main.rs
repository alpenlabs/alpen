//! Strata Test CLI
//!
//! Command-line utilities for Strata functional tests, providing:
//! - Bridge operations (deposit transaction, withdrawal fulfillment)
//! - Mock EE operations (mock deposit injection, snark withdrawal building)
//! - Schnorr signature operations
//! - Taproot address operations
//! - Public key aggregation (MuSig2)

use std::process;

mod bridge;
pub mod cmd;
mod constants;
mod error;
mod mock_ee;
mod parse;
mod schnorr;
mod taproot;
mod utils;

use cmd::{
    build_snark_withdrawal::build_snark_withdrawal,
    convert_to_xonly_pk::convert_to_xonly_pk, create_deposit_tx::create_deposit_tx,
    create_mock_deposit::create_mock_deposit,
    create_withdrawal_fulfillment::create_withdrawal_fulfillment,
    extract_p2tr_pubkey::extract_p2tr_pubkey, get_address::get_address,
    musig_aggregate_pks::musig_aggregate_pks, sign_schnorr_sig::sign_schnorr_sig,
    xonlypk_to_descriptor::xonlypk_to_descriptor, Commands, TopLevel,
};

fn main() {
    let TopLevel { cmd } = argh::from_env();

    let result = match cmd {
        Commands::CreateDepositTx(args) => create_deposit_tx(args),
        Commands::CreateWithdrawalFulfillment(args) => create_withdrawal_fulfillment(args),
        Commands::CreateMockDeposit(args) => create_mock_deposit(args),
        Commands::BuildSnarkWithdrawal(args) => build_snark_withdrawal(args),
        Commands::GetAddress(args) => get_address(args),
        Commands::MusigAggregatePks(args) => musig_aggregate_pks(args),
        Commands::ExtractP2trPubkey(args) => extract_p2tr_pubkey(args),
        Commands::ConvertToXonlyPk(args) => convert_to_xonly_pk(args),
        Commands::SignSchnorrSig(args) => sign_schnorr_sig(args),
        Commands::XonlypkToDescriptor(args) => xonlypk_to_descriptor(args),
    };

    if let Err(err) = result {
        eprintln!("{err}");
        process::exit(1);
    }
}
