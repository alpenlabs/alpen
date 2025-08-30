//! Alpen CLI

pub mod alpen;
pub mod cmd;
pub mod constants;
mod link;
pub mod net_type;
pub mod recovery;
pub mod seed;
pub mod settings;
pub mod signet;

use cmd::{
    backup::backup, balance::balance, config::config, deposit::deposit, drain::drain,
    faucet::faucet, receive::receive, recover::recover, scan::scan, send::send, withdraw::withdraw,
    Commands, TopLevel,
};
use settings::Settings;
use signet::persist::set_data_dir;

use crate::cmd::debug::debug;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let TopLevel { cmd } = argh::from_env();

    if let Commands::Config(args) = cmd {
        config(args).await;
        return;
    }

    let settings = Settings::load().unwrap_or_else(|e| {
        eprintln!("Configuration error: {e:?}");
        std::process::exit(1);
    });

    if let Commands::Reset(args) = cmd {
        let result = settings.secret_store.reset(args, &settings).await;
        if let Err(err) = result {
            eprintln!("{err}");
        }
        return;
    }

    assert!(set_data_dir(settings.data_dir.clone()));

    let seed = settings.secret_store.get_secret().unwrap_or_else(|e| {
        eprintln!("{e:?}");
        std::process::exit(1);
    });

    let result = match cmd {
        Commands::Recover(args) => recover(args, seed, settings).await,
        Commands::Drain(args) => drain(args, seed, settings).await,
        Commands::Balance(args) => balance(args, seed, settings).await,
        Commands::Backup(args) => backup(args, seed).await,
        Commands::Deposit(args) => deposit(args, seed, settings).await,
        Commands::Withdraw(args) => withdraw(args, seed, settings).await,
        Commands::Faucet(args) => faucet(args, seed, settings).await,
        Commands::Send(args) => send(args, seed, settings).await,
        Commands::Receive(args) => receive(args, seed, settings).await,
        Commands::ChangePwd(args) => settings.secret_store.change_pwd(args, seed).await,
        Commands::Scan(args) => scan(args, seed, settings).await,
        Commands::Debug(args) => debug(args, seed, settings).await,
        Commands::Config(_) => unreachable!("handled prior"),
        Commands::Reset(_) => unreachable!("handled prior"),
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
