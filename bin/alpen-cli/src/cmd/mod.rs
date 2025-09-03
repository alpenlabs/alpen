use argh::FromArgs;
use backup::BackupArgs;
use balance::BalanceArgs;
#[cfg(not(feature = "test-mode"))]
use change_pwd::ChangePwdArgs;
use config::ConfigArgs;
use deposit::DepositArgs;
use drain::DrainArgs;
use faucet::FaucetArgs;
use receive::ReceiveArgs;
use recover::RecoverArgs;
#[cfg(not(feature = "test-mode"))]
use reset::ResetArgs;
use scan::ScanArgs;
use send::SendArgs;
use withdraw::WithdrawArgs;

use crate::cmd::debug::DebugArgs;

pub mod backup;
pub mod balance;
pub mod change_pwd;
pub mod config;
pub mod debug;
pub mod deposit;
pub mod drain;
pub mod faucet;
pub mod receive;
pub mod recover;
pub mod reset;
pub mod scan;
pub mod send;
pub mod withdraw;

/// A CLI for interacting with Alpen and the underlying bitcoin (signet) network
#[derive(FromArgs, PartialEq, Debug)]
pub struct TopLevel {
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
    #[cfg(not(feature = "test-mode"))]
    ChangePwd(ChangePwdArgs),
    #[cfg(not(feature = "test-mode"))]
    Reset(ResetArgs),
    Scan(ScanArgs),
    Config(ConfigArgs),
    Debug(DebugArgs),
}
