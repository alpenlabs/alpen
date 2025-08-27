use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use strata_cli_common::errors::DisplayedError;
use terrors::OneOf;
#[cfg(feature = "test-mode")]
use zeroize::Zeroizing;

#[cfg(all(target_os = "linux", not(feature = "test-mode")))]
use super::FilePersister;
#[cfg(all(not(target_os = "linux"), not(feature = "test-mode")))]
use super::KeychainPersister;
#[cfg(not(feature = "test-mode"))]
use super::{load_or_create, EncryptedSeedPersister};
use super::{LoadOrCreateErr, Seed};
#[cfg(not(feature = "test-mode"))]
use crate::cmd::{change_pwd::change_pwd, reset::reset};
#[cfg(feature = "test-mode")]
use crate::settings::SettingsFromFile;
use crate::{
    cmd::{change_pwd::ChangePwdArgs, reset::ResetArgs},
    settings::Settings,
};

#[async_trait(?Send)]
pub trait SecretStore: Debug {
    fn get_secret(&self) -> Result<Seed, OneOf<LoadOrCreateErr>>;
    async fn reset(&self, args: ResetArgs, settings: &Settings) -> Result<(), DisplayedError>;
    async fn change_pwd(&self, args: ChangePwdArgs, seed: Seed) -> Result<(), DisplayedError>;
}

#[derive(Debug)]
#[cfg(not(feature = "test-mode"))]
pub struct UserSeedProvider {
    persister: Arc<dyn EncryptedSeedPersister>,
}

#[async_trait(?Send)]
#[cfg(not(feature = "test-mode"))]
impl SecretStore for UserSeedProvider {
    fn get_secret(&self) -> Result<Seed, OneOf<LoadOrCreateErr>> {
        load_or_create(self.persister.clone())
    }

    async fn reset(&self, args: ResetArgs, settings: &Settings) -> Result<(), DisplayedError> {
        reset(args, self.persister.clone(), settings).await
    }

    async fn change_pwd(&self, args: ChangePwdArgs, seed: Seed) -> Result<(), DisplayedError> {
        change_pwd(args, seed, self.persister.clone()).await
    }
}

#[cfg(feature = "test-mode")]
#[derive(Debug)]
pub struct TestSeedProvider {
    seed: Seed,
}

#[async_trait(?Send)]
#[cfg(feature = "test-mode")]
impl SecretStore for TestSeedProvider {
    fn get_secret(&self) -> Result<Seed, OneOf<LoadOrCreateErr>> {
        Ok(self.seed.clone())
    }

    async fn reset(&self, _args: ResetArgs, _settings: &Settings) -> Result<(), DisplayedError> {
        eprintln!("Reset is disabled for test mode");
        std::process::exit(1);
    }

    async fn change_pwd(&self, _args: ChangePwdArgs, _seed: Seed) -> Result<(), DisplayedError> {
        eprintln!("change password is disabled for test mode");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "test-mode"))]
pub fn secret_provider() -> Arc<dyn SecretStore> {
    #[cfg(not(target_os = "linux"))]
    let persister = KeychainPersister;
    #[cfg(target_os = "linux")]
    let persister = FilePersister::new(settings.linux_seed_file.clone());

    let usp = UserSeedProvider {
        persister: Arc::new(persister),
    };

    Arc::new(usp)
}

#[cfg(feature = "test-mode")]
pub fn secret_provider(settings: &SettingsFromFile) -> Arc<dyn SecretStore> {
    let bytes = &settings.seed;

    let test_provider = TestSeedProvider {
        seed: Seed(Zeroizing::new(**bytes)),
    };

    Arc::new(test_provider)
}
