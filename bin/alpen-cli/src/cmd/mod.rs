use argh::FromArgs;
use backup::BackupArgs;
use balance::BalanceArgs;
#[cfg(not(feature = "test-mode"))]
use change_pwd::ChangePwdArgs;
use config::ConfigArgs;
use deposit::DepositArgs;
use drain::DrainArgs;
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
pub mod receive;
pub mod recover;
pub mod reset;
pub mod scan;
pub mod send;
pub mod withdraw;

/// A CLI for interacting with Alpen and its underlying Bitcoin network.
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

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "windows"))]
    use std::fs::remove_dir_all;
    use std::{
        env::{set_var, temp_dir},
        fs::{create_dir_all, write},
        process,
        sync::Arc,
    };

    use argh::FromArgs;
    use bdk_wallet::bitcoin::{FeeRate, Network};
    use shrex::Hex;

    use super::*;
    use crate::{
        bitcoin::{persist::set_data_dir, tests::TestBitcoinBackend},
        constants::SEED_LEN,
        seed::Seed,
        settings::Settings,
    };

    fn seed() -> Seed {
        Seed::from_file(Hex([0; SEED_LEN]))
    }

    #[tokio::test]
    async fn mainnet_commands_work_offline() {
        let test_root = temp_dir().join(format!("alpen-cli-mainnet-{}", process::id()));
        let config_file = test_root.join("config.toml");
        create_dir_all(&test_root).expect("test directory should be created");
        write(
            &config_file,
            r#"
                esplora = "https://esplora.example.com"
                alpen_endpoint = "https://rpc.example.com"
                bridge_pubkey = "1d3e9c0417ba7d3551df5a1cc1dbe227aa4ce89161762454d92bfc2b1d5886f7"
                network = "bitcoin"
                magic_bytes = "STRA"
                bridge_denomination_sats = 200000000
                recovery_delay = 36
                max_withdrawal_descriptor_len = 81
                seed = "000102030405060708090a0b0c0d0e0f"
            "#,
        )
        .expect("test config should be written");
        set_var("PROJ_DIRS", &test_root);
        set_var("CLI_CONFIG", &config_file);
        assert!(set_data_dir(test_root.clone(), Network::Bitcoin));

        let backend = Arc::new(TestBitcoinBackend {
            fee_rate: Some(FeeRate::BROADCAST_MIN),
        });
        let load_settings = || {
            let mut settings = Settings::load().expect("mainnet settings should load");
            assert_eq!(settings.network, Network::Bitcoin);
            settings.bitcoin_backend = backend.clone();
            settings
        };

        let args = ReceiveArgs::from_args(&["alpen", "receive"], &["bitcoin"]).unwrap();
        receive::receive(args, seed(), load_settings())
            .await
            .unwrap();
        let args = BalanceArgs::from_args(&["alpen", "balance"], &["bitcoin"]).unwrap();
        balance::balance(args, seed(), load_settings())
            .await
            .unwrap();
        let args = ScanArgs::from_args(&["alpen", "scan"], &[]).unwrap();
        scan::scan(args, seed(), load_settings()).await.unwrap();
        let args = RecoverArgs::from_args(&["alpen", "recover"], &[]).unwrap();
        recover::recover(args, seed(), load_settings())
            .await
            .unwrap();
        let args = DepositArgs::from_args(&["alpen", "deposit"], &[]).unwrap();
        assert!(deposit::deposit(args, seed(), load_settings())
            .await
            .is_err());
        let address = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";
        let args =
            DrainArgs::from_args(&["alpen", "drain"], &["--bitcoin-address", address]).unwrap();
        assert!(drain::drain(args, seed(), load_settings()).await.is_err());
        let args = SendArgs::from_args(&["alpen", "send"], &["bitcoin", "0", address]).unwrap();
        assert!(send::send(args, seed(), load_settings()).await.is_err());

        let args = WithdrawArgs::from_args(&["alpen", "withdraw"], &[]).unwrap();
        let mut settings = load_settings();
        settings.alpen_endpoint = "not a URL".into();
        assert!(withdraw::withdraw(args, seed(), settings).await.is_err());

        #[cfg(not(target_os = "windows"))]
        remove_dir_all(test_root).expect("test directory should be removed");
    }
}
