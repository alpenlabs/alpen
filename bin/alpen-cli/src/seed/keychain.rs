use keyring::{Credential, Entry, Error};
use terrors::OneOf;

use super::{EncryptedSeed, EncryptedSeedPersister, PersisterErr};

#[derive(Debug, Clone, Copy)]
pub struct KeychainPersister;

impl KeychainPersister {
    fn entry() -> Result<Entry, OneOf<(PlatformFailure, NoStorageAccess)>> {
        Entry::new("alpen", "default")
            .map_err(keyring_oneof)
            .map_err(|e| {
                e.subset()
                    .unwrap_or_else(|e| panic!("errored subsetting keychain error: {e:?}"))
            })
    }
}

impl EncryptedSeedPersister for KeychainPersister {
    fn save(&self, seed: &EncryptedSeed) -> Result<(), PersisterErr> {
        let entry = Self::entry()?;
        entry
            .set_secret(&seed.0)
            .map_err(keyring_oneof)
            .map_err(|e| {
                e.subset()
                    .unwrap_or_else(|e| panic!("errored subsetting keychain error: {e:?}"))
            })
    }

    fn load(&self) -> Result<Option<EncryptedSeed>, PersisterErr> {
        let entry = Self::entry()?;

        let secret = match entry.get_secret().map_err(keyring_oneof) {
            Ok(s) => s,
            Err(e) => {
                let no_entry = e.narrow::<NoEntry, _>();
                if no_entry.is_ok() {
                    return Ok(None);
                }

                let bad_encoding = no_entry.unwrap_err().narrow::<BadEncoding, _>();
                if bad_encoding.is_ok() {
                    let _ = entry.delete_credential();
                    return Ok(None);
                }

                return Err(bad_encoding
                    .unwrap_err()
                    .subset()
                    .unwrap_or_else(|e| panic!("errored subsetting keychain error: {e:?}")));
            }
        };

        if secret.len() == EncryptedSeed::LEN {
            Ok(Some(EncryptedSeed(secret.try_into().unwrap())))
        } else {
            let _ = entry.delete_credential();
            Ok(None)
        }
    }

    fn delete(&self) -> Result<(), PersisterErr> {
        let entry = Self::entry().map_err(OneOf::broaden)?;
        if let Err(e) = entry.delete_credential().map_err(keyring_oneof) {
            // if e is NOT a NoEntry error
            if let Err(e) = e.narrow::<NoEntry, _>() {
                panic!("bad error: {e:?}")
            }
        }
        Ok(())
    }
}

use crate::errors::{NoStorageAccess, PlatformFailure};

// below is wrapper around [`keyring::Error`] so it can be used with OneOf to more precisely handle
// errors

/// This indicates that there is no underlying credential entry in the platform for this entry.
/// Either one was never set, or it was deleted.
#[derive(Debug)]
pub struct NoEntry;

/// This indicates that the retrieved password blob was not a UTF-8 string. The underlying bytes are
/// available for examination in the attached value.
#[derive(Debug)]
#[allow(unused)]
pub struct BadEncoding(Vec<u8>);

/// This indicates that one of the entry's credential attributes exceeded a length limit in the
/// underlying platform. The attached values give the name of the attribute and the platform length
/// limit that was exceeded.
#[derive(Debug)]
#[allow(unused)]
pub struct TooLong {
    name: String,
    limit: u32,
}

/// This indicates that one of the entry's required credential attributes was invalid. The attached
/// value gives the name of the attribute and the reason it's invalid.
#[derive(Debug)]
#[allow(unused)]
pub struct Invalid {
    name: String,
    reason: String,
}

/// This indicates that there is more than one credential found in the store that matches the entry.
/// Its value is a vector of the matching credentials.
#[derive(Debug)]
#[allow(unused)]
pub struct Ambiguous(Vec<Box<Credential>>);

#[cfg(not(target_os = "linux"))]
type KeyRingErrors = (
    PlatformFailure,
    NoStorageAccess,
    NoEntry,
    BadEncoding,
    TooLong,
    Invalid,
    Ambiguous,
);

fn keyring_oneof(err: keyring::Error) -> OneOf<KeyRingErrors> {
    match err {
        Error::PlatformFailure(error) => OneOf::new(PlatformFailure::new(error)),
        Error::NoStorageAccess(error) => OneOf::new(NoStorageAccess::new(error)),
        Error::NoEntry => OneOf::new(NoEntry),
        Error::BadEncoding(vec) => OneOf::new(BadEncoding(vec)),
        Error::TooLong(name, limit) => OneOf::new(TooLong { name, limit }),
        Error::Invalid(name, reason) => OneOf::new(Invalid { name, reason }),
        Error::Ambiguous(vec) => OneOf::new(Ambiguous(vec)),
        _ => todo!(),
    }
}
