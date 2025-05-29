//! Alpen CLI

pub mod alpen;
pub mod cmd;
pub mod constants;
#[macro_use]
pub mod errors;
mod link;
pub mod net_type;
pub mod recovery;
pub mod seed;
pub mod settings;
pub mod signet;
pub mod taproot;

use cmd::{
    backup::backup, balance::balance, change_pwd::change_pwd, config::config, deposit::deposit,
    drain::drain, faucet::faucet, receive::receive, recover::recover, reset::reset, scan::scan,
    send::send, withdraw::withdraw, Commands, TopLevel,
};
#[cfg(target_os = "linux")]
use seed::FilePersister;
#[cfg(not(target_os = "linux"))]
use seed::KeychainPersister;
use settings::Settings;
use signet::persist::set_data_dir;

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

    #[cfg(not(target_os = "linux"))]
    let persister = KeychainPersister;
    #[cfg(target_os = "linux")]
    let persister = FilePersister::new(settings.linux_seed_file.clone());

    if let Commands::Reset(args) = cmd {
        let result = reset(args, persister, settings).await;
        if let Err(err) = result {
            eprintln!("{err}");
        }
        return;
    }

    assert!(set_data_dir(settings.data_dir.clone()));

    let seed = seed::load_or_create(&persister).unwrap_or_else(|e| {
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
        Commands::ChangePwd(args) => change_pwd(args, seed, persister).await,
        Commands::Scan(args) => scan(args, seed, settings).await,
        _ => Ok(()),
    };

    if let Err(err) = result {
        eprintln!("{err}");
    }
}
