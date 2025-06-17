use argh::FromArgs;
use backup::BackupArgs;
use balance::BalanceArgs;
use change_pwd::ChangePwdArgs;
use config::ConfigArgs;
use deposit::DepositArgs;
use drain::DrainArgs;
use faucet::FaucetArgs;
use generate_params::GenerateParamsArgs;
use receive::ReceiveArgs;
use recover::RecoverArgs;
use reset::ResetArgs;
use scan::ScanArgs;
use send::SendArgs;
use withdraw::WithdrawArgs;

pub mod backup;
pub mod balance;
pub mod change_pwd;
pub mod config;
pub mod deposit;
pub mod drain;
pub mod faucet;
pub mod generate_params;
pub mod receive;
pub mod recover;
pub mod reset;
pub mod scan;
pub mod send;
pub mod withdraw;

/// A CLI for interacting with Alpen and the underlying bitcoin (signet) network
#[derive(FromArgs, PartialEq, Debug)]
pub struct TopLevel {
    /// custom protocol parameters file
    #[argh(option)]
    pub protocol_params: Option<std::path::PathBuf>,

    #[argh(subcommand)]
    pub cmd: Commands,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub enum Commands {
    Recover(RecoverArgs),
    Drain(DrainArgs),
    Balance(BalanceArgs),
    Backup(BackupArgs),
    Deposit(DepositArgs),
    Withdraw(WithdrawArgs),
    Faucet(FaucetArgs),
    Send(SendArgs),
    Receive(ReceiveArgs),
    ChangePwd(ChangePwdArgs),
    Reset(ResetArgs),
    Scan(ScanArgs),
    Config(ConfigArgs),
    GenerateParams(GenerateParamsArgs),
}
