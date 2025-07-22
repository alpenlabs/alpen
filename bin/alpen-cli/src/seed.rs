#[cfg(target_os = "linux")]
use std::io;
use std::str::FromStr;

use aes_gcm_siv::{aead::AeadMutInPlace, Aes256GcmSiv, KeyInit, Nonce, Tag};
use alloy::{network::EthereumWallet, signers::local::PrivateKeySigner};
use bdk_wallet::{
    bitcoin::{
        bip32::{DerivationPath, Xpriv},
        secp256k1::SECP256K1,
        Network,
    },
    CreateParams, KeychainKind, LoadParams, Wallet,
};
use bip39::{Language, Mnemonic};
use dialoguer::{Confirm, Input};
use password::{HashVersion, IncorrectPassword, Password};
use rand_core::{CryptoRngCore, OsRng};
use sha2::{Digest, Sha256};
use terrors::OneOf;
use zeroize::Zeroizing;

use crate::constants::{
    AES_NONCE_LEN, AES_TAG_LEN, BIP44_ALPEN_EVM_WALLET_PATH, PW_SALT_LEN, SEED_LEN,
};
#[cfg(not(target_os = "linux"))]
use crate::errors::{NoStorageAccess, PlatformFailure};

pub struct BaseWallet(LoadParams, CreateParams);

impl BaseWallet {
    pub fn split(self) -> (LoadParams, CreateParams) {
        (self.0, self.1)
    }
}

#[derive(Clone)]
// NOTE: This is not a BIP39 seed, instead random bytes of entropy.
pub struct Seed(Zeroizing<[u8; SEED_LEN]>);

impl Seed {
    fn gen<R: CryptoRngCore>(rng: &mut R) -> Self {
        let mut bytes = Zeroizing::new([0u8; SEED_LEN]);
        rng.fill_bytes(bytes.as_mut());
        Self(bytes)
    }

    pub fn print_mnemonic(&self, language: Language) {
        let mnemonic = Mnemonic::from_entropy_in(language, self.0.as_ref()).expect("valid entropy");
        println!("{mnemonic}");
    }

    pub fn descriptor_recovery_key(&self) -> [u8; 32] {
        let mut hasher = <Sha256 as Digest>::new(); // this is to appease the analyzer
        hasher.update(b"alpen labs alpen descriptor recovery file 2024");
        hasher.update(self.0.as_slice());
        hasher.finalize().into()
    }

    pub fn encrypt<R: CryptoRngCore>(
        &self,
        password: &mut Password,
        rng: &mut R,
    ) -> Result<EncryptedSeed, OneOf<(argon2::Error, aes_gcm_siv::Error)>> {
        let mut buf = [0u8; EncryptedSeed::LEN];
        rng.fill_bytes(&mut buf[..PW_SALT_LEN + AES_NONCE_LEN]);

        let seed_encryption_key = password
            .seed_encryption_key(
                &buf[..PW_SALT_LEN].try_into().expect("cannot fail"),
                HashVersion::V0,
            )
            .map_err(OneOf::new)?;

        let (salt_and_nonce, rest) = buf.split_at_mut(PW_SALT_LEN + AES_NONCE_LEN);
        let (seed, _) = rest.split_at_mut(SEED_LEN);
        seed.copy_from_slice(self.0.as_ref());

        let mut cipher = Aes256GcmSiv::new_from_slice(seed_encryption_key.as_ref())
            .expect("should be correct key size");
        let nonce = Nonce::from_slice(&salt_and_nonce[PW_SALT_LEN..]);
        let tag = cipher
            .encrypt_in_place_detached(nonce, &[], seed)
            .map_err(OneOf::new)?;
        buf[(EncryptedSeed::LEN - AES_TAG_LEN)..].copy_from_slice(tag.as_slice());
        Ok(EncryptedSeed(buf))
    }

    pub fn signet_wallet(&self) -> BaseWallet {
        let rootpriv = Xpriv::new_master(Network::Signet, self.0.as_ref()).expect("valid xpriv");
        let base_desc = format!("tr({rootpriv}/86h/0h/0h");
        let external_desc = format!("{base_desc}/0/*)");
        let internal_desc = format!("{base_desc}/1/*)");
        BaseWallet(
            Wallet::load()
                .descriptor(KeychainKind::External, Some(external_desc.clone()))
                .descriptor(KeychainKind::Internal, Some(internal_desc.clone()))
                .extract_keys(),
            Wallet::create(external_desc, internal_desc),
        )
    }

    pub fn get_alpen_wallet(&self) -> EthereumWallet {
        let derivation_path = DerivationPath::master().extend(BIP44_ALPEN_EVM_WALLET_PATH);

        let mnemonic = Mnemonic::from_entropy(self.0.as_ref()).expect("valid entropy");
        // We do not use a passphrase.
        let bip39_seed = mnemonic.to_seed("");
        // Network choice affects how extended public and private keys are serialized. See
        // https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki#serialization-format.
        // Given the popularity of MetaMask, we follow their example (they always hardcode mainnet)
        // and hardcode Network::Bitcoin (mainnet) for EVM-based wallet.
        let master_key = Xpriv::new_master(Network::Bitcoin, &bip39_seed).expect("valid xpriv");

        // Derive the child key for the given path
        let derived_key = master_key.derive_priv(SECP256K1, &derivation_path).unwrap();
        let signer =
            PrivateKeySigner::from_slice(derived_key.private_key.secret_bytes().as_slice())
                .expect("valid slice");

        EthereumWallet::from(signer)
    }
}

pub struct EncryptedSeed([u8; Self::LEN]);

impl EncryptedSeed {
    const LEN: usize = PW_SALT_LEN + AES_NONCE_LEN + SEED_LEN + AES_TAG_LEN;

    fn decrypt(
        mut self,
        password: &mut Password,
    ) -> Result<Seed, OneOf<(argon2::Error, aes_gcm_siv::Error)>> {
        let seed_encryption_key = password
            .seed_encryption_key(
                &self.0[..PW_SALT_LEN].try_into().expect("cannot fail"),
                HashVersion::V0,
            )
            .map_err(OneOf::new)?;

        let mut cipher = Aes256GcmSiv::new_from_slice(seed_encryption_key.as_ref())
            .expect("should be correct key size");
        let (salt_and_nonce, rest) = self.0.split_at_mut(PW_SALT_LEN + AES_NONCE_LEN);
        let (encrypted_seed, tag) = rest.split_at_mut(SEED_LEN);
        let tag = Tag::from_slice(tag);
        let nonce = Nonce::from_slice(&salt_and_nonce[PW_SALT_LEN..]);

        let mut seed = Zeroizing::new([0u8; SEED_LEN]);
        seed.copy_from_slice(encrypted_seed);

        cipher
            .decrypt_in_place_detached(nonce, &[], seed.as_mut(), tag)
            .map_err(OneOf::new)?;

        Ok(Seed(seed))
    }
}

pub fn load_or_create(
    persister: &impl EncryptedSeedPersister,
) -> Result<Seed, OneOf<LoadOrCreateErr>> {
    println!("Loading encrypted seed...");
    let maybe_encrypted_seed = persister.load().map_err(OneOf::broaden)?;
    if let Some(encrypted_seed) = maybe_encrypted_seed {
        println!("Opening wallet...");
        let mut password = Password::read(false).map_err(OneOf::new)?;
        match encrypted_seed.decrypt(&mut password) {
            Ok(seed) => {
                println!("Wallet unlocked");
                Ok(seed)
            }
            Err(e) => {
                let narrowed = e.narrow::<aes_gcm_siv::Error, _>();
                if let Ok(_aes_error) = narrowed {
                    return Err(OneOf::new(IncorrectPassword));
                }

                Err(narrowed.unwrap_err().broaden())
            }
        }
    } else {
        let restore = Confirm::new()
            .with_prompt("Do you want to restore a previously created wallet?")
            .interact()
            .map_err(OneOf::new)?;

        let seed = if restore {
            loop {
                let mnemonic: String = Input::new()
                    .with_prompt("Enter your mnemonic")
                    .interact_text()
                    .map_err(OneOf::new)?;

                let mnemonic = match Mnemonic::from_str(&mnemonic) {
                    Ok(m) => m,
                    Err(e) => {
                        println!("please try again: {e}");
                        continue;
                    }
                };
                let entropy = mnemonic.to_entropy();
                if entropy.len() != SEED_LEN {
                    println!("incorrect entropy length");
                    continue;
                }
                let mut buf = Zeroizing::new([0u8; SEED_LEN]);
                buf.copy_from_slice(&entropy);
                break Seed(buf);
            }
        } else {
            println!("Creating new wallet");
            Seed::gen(&mut OsRng)
        };

        let mut password = Password::read(true).map_err(OneOf::new)?;
        let password_validation: Result<(), String> = password.validate();
        if let Err(feedback) = password_validation {
            println!("Password is weak. {feedback}");
        };
        let encrypted_seed = match seed.encrypt(&mut password, &mut OsRng) {
            Ok(es) => es,
            Err(e) => {
                let narrowed = e.narrow::<aes_gcm_siv::Error, _>();
                if let Ok(aes_error) = narrowed {
                    panic!("Failed to encrypt seed: {aes_error:?}");
                }

                return Err(narrowed.unwrap_err().broaden());
            }
        };
        persister.save(&encrypted_seed).map_err(OneOf::broaden)?;
        Ok(seed)
    }
}

#[cfg(not(target_os = "linux"))]
type PersisterErr = OneOf<(PlatformFailure, NoStorageAccess)>;

#[cfg(target_os = "linux")]
type PersisterErr = OneOf<(io::Error,)>;

#[cfg(target_os = "linux")]
type LoadOrCreateErr = (
    io::Error,
    dialoguer::Error,
    argon2::Error,
    IncorrectPassword,
);

#[cfg(not(target_os = "linux"))]
type LoadOrCreateErr = (
    PlatformFailure,
    NoStorageAccess,
    dialoguer::Error,
    argon2::Error,
    IncorrectPassword,
);

pub trait EncryptedSeedPersister {
    fn save(&self, seed: &EncryptedSeed) -> Result<(), PersisterErr>;
    fn load(&self) -> Result<Option<EncryptedSeed>, PersisterErr>;
    fn delete(&self) -> Result<(), PersisterErr>;
}

#[cfg(target_os = "linux")]
pub use file::*;

#[cfg(target_os = "linux")]
mod file;

#[cfg(not(target_os = "linux"))]
mod keychain;

#[cfg(not(target_os = "linux"))]
pub use keychain::*;

pub mod password;

#[cfg(test)]
mod test {
    use rand_core::OsRng;
    use sha2::digest::generic_array::GenericArray;

    use super::*;

    #[test]
    // Sanity checks on curve scalar construction, to ensure proper rejection
    // This treats zero as invalid (for ECDSA reasons)
    fn scalar_sanity_checks() {
        // This is the (big-endian) order of the `secp256k1` curve group
        // You can find it in, for example, section 2.4.1 of https://www.secg.org/sec2-v2.pdf
        let mut order: [u8; 32] = [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x41,
        ];

        // The scalar can't be zero
        assert!(PrivateKeySigner::from_field_bytes(GenericArray::from_slice(&[0u8; 32])).is_err());

        // The scalar can be well within the group order
        assert!(PrivateKeySigner::from_field_bytes(GenericArray::from_slice(&[1u8; 32])).is_ok());

        // The scalar can't equal the group order
        assert!(PrivateKeySigner::from_field_bytes(GenericArray::from_slice(&order)).is_err());

        // The scalar can't exceed the group order
        order[31] = 0x42;
        assert!(PrivateKeySigner::from_field_bytes(GenericArray::from_slice(&order)).is_err());
        assert!(
            PrivateKeySigner::from_field_bytes(GenericArray::from_slice(&[u8::MAX; 32])).is_err()
        );

        // The scalar can be _just_ under the group order
        order[31] = 0x40;
        assert!(PrivateKeySigner::from_field_bytes(GenericArray::from_slice(&order)).is_ok());
    }

    #[test]
    // Test valid seed encryption and decryption
    fn seed_encrypt_decrypt() {
        let mut password = Password::new(String::from("swordfish"));
        let seed = Seed::gen(&mut OsRng);

        let encrypted_seed = seed.encrypt(&mut password, &mut OsRng).unwrap();
        let decrypted_seed = encrypted_seed.decrypt(&mut password).unwrap();

        assert_eq!(seed.0, decrypted_seed.0);
    }

    #[test]
    // Using an evil password fails decryption
    fn evil_password() {
        let mut password = Password::new(String::from("swordfish"));
        let mut evil_password = Password::new(String::from("evil"));
        let seed = Seed::gen(&mut OsRng);

        let encrypted_seed = seed.encrypt(&mut password, &mut OsRng).unwrap();

        assert!(encrypted_seed.decrypt(&mut evil_password).is_err());
    }

    #[test]
    // Using an evil salt fails decryption
    fn evil_salt() {
        let mut password = Password::new(String::from("swordfish"));
        let seed = Seed::gen(&mut OsRng);

        let mut encrypted_seed = seed.encrypt(&mut password, &mut OsRng).unwrap();
        let index = 0;
        encrypted_seed.0[index] = !encrypted_seed.0[index];

        assert!(encrypted_seed.decrypt(&mut password).is_err());
    }

    #[test]
    // Using an evil nonce fails decryption
    fn evil_nonce() {
        let mut password = Password::new(String::from("swordfish"));
        let seed = Seed::gen(&mut OsRng);

        let mut encrypted_seed = seed.encrypt(&mut password, &mut OsRng).unwrap();
        let index = PW_SALT_LEN;
        encrypted_seed.0[index] = !encrypted_seed.0[index];

        assert!(encrypted_seed.decrypt(&mut password).is_err());
    }

    #[test]
    // Using an evil seed fails decryption
    fn evil_seed() {
        let mut password = Password::new(String::from("swordfish"));
        let seed = Seed::gen(&mut OsRng);

        let mut encrypted_seed = seed.encrypt(&mut password, &mut OsRng).unwrap();
        let index = PW_SALT_LEN + AES_NONCE_LEN;
        encrypted_seed.0[index] = !encrypted_seed.0[index];

        assert!(encrypted_seed.decrypt(&mut password).is_err());
    }

    #[test]
    // Using an evil tag fails decryption
    fn evil_tag() {
        let mut password = Password::new(String::from("swordfish"));
        let seed = Seed::gen(&mut OsRng);

        let mut encrypted_seed = seed.encrypt(&mut password, &mut OsRng).unwrap();
        let index = PW_SALT_LEN + AES_NONCE_LEN + SEED_LEN;
        encrypted_seed.0[index] = !encrypted_seed.0[index];

        assert!(encrypted_seed.decrypt(&mut password).is_err());
    }

    #[test]
    // Test L2 wallet address matches the one from BIP39 tool (e.g. https://iancoleman.io/bip39/)
    // using the same menmonic and derivation path.
    fn test_l2_wallet_address() {
        let seed = Seed(
            [
                0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36,
                0x41, 0x41,
            ]
            .into(),
        );
        let l2wallet = seed.get_alpen_wallet();
        let address = l2wallet.default_signer().address().to_string();
        // BIP39 Mnemonic for `seed` should be:
        // rival ivory defy future meat build young envelope mimic like motion loan
        // The expected address is obtained using the BIP39 tool with the above mnemonic
        // and BIP44 derivation path m/44'/60'/0'/0/0.
        let expected_address = "0x4eEE6B504Bc2c47650bAa7d135DA10F2A469E582".to_string();
        assert_eq!(address, expected_address);
    }
}
