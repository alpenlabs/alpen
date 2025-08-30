//! Abstraction over secret storage with platform-specific providers (file, keychain, or test).

use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use strata_cli_common::errors::DisplayedError;
use terrors::OneOf;

use super::{LoadOrCreateErr, Seed};
use crate::{
    cmd::{change_pwd::ChangePwdArgs, reset::ResetArgs},
    settings::Settings,
};

/// Defines a common interface for retrieving, resetting, and updating encrypted seeds.
#[async_trait(?Send)]
pub trait SecretStore: Debug {
    fn get_secret(&self) -> Result<Seed, OneOf<LoadOrCreateErr>>;
    async fn reset(&self, args: ResetArgs, settings: &Settings) -> Result<(), DisplayedError>;
    async fn change_pwd(&self, args: ChangePwdArgs, seed: Seed) -> Result<(), DisplayedError>;
}

#[cfg(not(feature = "test-mode"))]
mod impls {
    use std::path::Path;

    use super::*;
    #[cfg(target_os = "linux")]
    use crate::seed::FilePersister;
    #[cfg(not(target_os = "linux"))]
    use crate::seed::KeychainPersister;
    use crate::{
        cmd::{change_pwd::change_pwd, reset::reset},
        seed::{load_or_create, EncryptedSeedPersister},
    };

    /// Provides access to a user’s seed using an encrypted persister (file or keychain).
    #[derive(Debug)]
    pub(super) struct UserSeedProvider<P: EncryptedSeedPersister + Clone + 'static> {
        pub(super) persister: P,
    }

    #[async_trait(?Send)]
    impl<P: EncryptedSeedPersister + Clone + 'static> SecretStore for UserSeedProvider<P> {
        fn get_secret(&self) -> Result<Seed, OneOf<LoadOrCreateErr>> {
            load_or_create(&self.persister)
        }

        async fn reset(&self, args: ResetArgs, settings: &Settings) -> Result<(), DisplayedError> {
            reset(args, &self.persister, settings).await
        }

        async fn change_pwd(&self, args: ChangePwdArgs, seed: Seed) -> Result<(), DisplayedError> {
            change_pwd(args, seed, &self.persister).await
        }
    }

    /// Creates a platform-appropriate persister (file on Linux, keychain otherwise).
    #[cfg(target_os = "linux")]
    fn make_persister(seed_file: &Path) -> FilePersister {
        FilePersister::new(seed_file.to_owned())
    }

    #[cfg(not(target_os = "linux"))]
    fn make_persister(_seed_file: &Path) -> KeychainPersister {
        KeychainPersister
    }

    /// Returns a secret provider backed by the platform’s encrypted seed persister
    pub fn secret_provider(seed_file: &Path) -> Arc<dyn SecretStore> {
        let persister = make_persister(seed_file);
        Arc::new(UserSeedProvider { persister })
    }
}
#[cfg(not(feature = "test-mode"))]
pub use impls::secret_provider;

#[cfg(feature = "test-mode")]
mod test_impls {
    use zeroize::Zeroizing;

    use super::*;
    use crate::settings::SettingsFromFile;

    /// Test-mode seed provider that returns a fixed seed and exits on reset or password change.
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

    /// Returns a secret provider that uses in memory seed from test
    pub fn secret_provider(settings: &SettingsFromFile) -> Arc<dyn SecretStore> {
        let bytes = &settings.seed;
        let seed = Seed(Zeroizing::new(**bytes));
        Arc::new(TestSeedProvider { seed })
    }
}
#[cfg(feature = "test-mode")]
pub use test_impls::secret_provider;
