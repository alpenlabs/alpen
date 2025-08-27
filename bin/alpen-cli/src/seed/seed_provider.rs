use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use strata_cli_common::errors::DisplayedError;
use terrors::OneOf;
#[cfg(feature = "test-mode")]
use zeroize::Zeroizing;

use super::{LoadOrCreateErr, Seed};
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

#[cfg(not(feature = "test-mode"))]
mod impls {
    use std::path::Path;

    use super::{
        super::{load_or_create, EncryptedSeedPersister},
        *,
    };
    use crate::cmd::{change_pwd::change_pwd, reset::reset};

    type DynPersister = Arc<dyn EncryptedSeedPersister>;

    #[derive(Debug)]
    pub(super) struct UserSeedProvider {
        pub(super) persister: DynPersister,
    }

    #[async_trait(?Send)]
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

    #[cfg(target_os = "linux")]
    fn make_persister(seed_file: &Path) -> DynPersister {
        let p = super::super::FilePersister::new(seed_file.to_owned());
        Arc::new(p)
    }

    #[cfg(not(target_os = "linux"))]
    fn make_persister(_seed_file: &Path) -> DynPersister {
        let p = super::super::KeychainPersister;
        Arc::new(p)
    }

    pub fn secret_provider(seed_file: &Path) -> Arc<dyn SecretStore> {
        let persister = make_persister(seed_file);
        Arc::new(UserSeedProvider { persister })
    }
}
#[cfg(not(feature = "test-mode"))]
pub use impls::secret_provider;

#[cfg(feature = "test-mode")]
mod test_impls {
    use super::*;
    use crate::settings::SettingsFromFile;

    #[derive(Debug, Clone)]
    pub(super) struct TestSeedProvider {
        pub(super) seed: Seed,
    }

    #[async_trait(?Send)]
    impl SecretStore for TestSeedProvider {
        fn get_secret(&self) -> Result<Seed, OneOf<LoadOrCreateErr>> {
            Ok(self.seed.clone())
        }

        async fn reset(
            &self,
            _args: ResetArgs,
            _settings: &Settings,
        ) -> Result<(), DisplayedError> {
            eprintln!("Reset is disabled for test mode");
            std::process::exit(1);
        }

        async fn change_pwd(
            &self,
            _args: ChangePwdArgs,
            _seed: Seed,
        ) -> Result<(), DisplayedError> {
            eprintln!("change password is disabled for test mode");
            std::process::exit(1);
        }
    }

    pub fn secret_provider(settings: &SettingsFromFile) -> Arc<dyn SecretStore> {
        let bytes = &settings.seed;
        let seed = Seed(Zeroizing::new(**bytes));
        Arc::new(TestSeedProvider { seed })
    }
}
#[cfg(feature = "test-mode")]
pub use test_impls::secret_provider;
