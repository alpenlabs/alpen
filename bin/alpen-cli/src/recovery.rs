use std::{
    collections::{BTreeMap, HashSet},
    io::{self, Cursor, Read},
    ops::{Bound, RangeBounds},
    path::Path,
    str::FromStr,
    string::FromUtf8Error,
};

use aes_gcm_siv::{aead::AeadMutInPlace, Aes256GcmSiv, KeyInit, Nonce, Tag};
use bdk_wallet::{
    bitcoin::{constants::ChainHash, Network},
    keys::{DescriptorPublicKey, DescriptorSecretKey},
    miniscript::{self, descriptor::DescriptorKeyParseError, Descriptor},
    template::DescriptorTemplateOut,
};
use make_buf::make_buf;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use terrors::OneOf;
use tokio::io::AsyncReadExt;

use crate::seed::Seed;

// Key for the reclaim counter within its own tree (see `counter_tree` below) -- any fixed key
// works here since this tree holds nothing else.
const RECLAIM_COUNTER_KEY: &[u8] = b"drt_reclaim_counter";

#[expect(
    missing_debug_implementations,
    reason = "Struct contains sensitive cryptographic data that should not be debug printed"
)]
pub struct DescriptorRecovery {
    db: sled::Db,
    // Reclaim-counter state lives in its own sled tree, not the default tree `db` derefs to,
    // so it can never show up in a `read_descs` range scan over descriptor entries -- which,
    // being always exactly `DescriptorRecoveryKey::ENCODED_SIZE` bytes, would otherwise panic
    // on `DescriptorRecoveryKey::decode(&key).unwrap()` the first time it saw this key.
    counter_tree: sled::Tree,
    cipher: Aes256GcmSiv,
}

impl DescriptorRecovery {
    /// Reserves and returns the next counter for deriving a deposit-request reclaim key (see
    /// [`Seed::drt_reclaim_keypair`]).
    ///
    /// The increment is flushed to disk before this returns, so a crash right after can't cause
    /// the same counter to be handed out twice. Missing counter state is an error: callers must
    /// first reconstruct or initialize it with [`Self::ensure_reclaim_counter_at_least`].
    pub async fn next_reclaim_counter(&self) -> io::Result<u32> {
        let updated = self
            .counter_tree
            .update_and_fetch(RECLAIM_COUNTER_KEY, |old| {
                old.map(|bytes| {
                    let current =
                        u32::from_be_bytes(bytes.try_into().expect("4-byte reclaim counter"));
                    (current + 1).to_be_bytes().to_vec()
                })
            })?;
        let updated = updated.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "deposit reclaim counter has not been reconstructed",
            )
        })?;
        self.counter_tree.flush_async().await?;
        Ok(u32::from_be_bytes(
            updated.as_ref().try_into().expect("4-byte reclaim counter"),
        ))
    }

    /// Advances the reclaim counter to at least `minimum` and flushes it to disk.
    ///
    /// This initializes missing counter state after seed recovery without rolling back a higher
    /// value another process may already have persisted.
    pub async fn ensure_reclaim_counter_at_least(&self, minimum: u32) -> io::Result<u32> {
        let updated = self
            .counter_tree
            .update_and_fetch(RECLAIM_COUNTER_KEY, |old| {
                let current = old.map(|bytes| {
                    u32::from_be_bytes(bytes.try_into().expect("4-byte reclaim counter"))
                });
                Some(
                    current
                        .map_or(minimum, |value| value.max(minimum))
                        .to_be_bytes()
                        .to_vec(),
                )
            })?
            .expect("update_and_fetch always writes a value");
        self.counter_tree.flush_async().await?;
        Ok(u32::from_be_bytes(
            updated.as_ref().try_into().expect("4-byte reclaim counter"),
        ))
    }

    /// Returns the last counter [`Self::next_reclaim_counter`] handed out, if allocator state has
    /// been initialized. For inspection, not for deriving a new deposit's key.
    pub fn current_reclaim_counter(&self) -> io::Result<Option<u32>> {
        Ok(self.counter_tree.get(RECLAIM_COUNTER_KEY)?.map(|bytes| {
            u32::from_be_bytes(bytes.as_ref().try_into().expect("4-byte reclaim counter"))
        }))
    }

    pub async fn add_desc(
        &mut self,
        recover_at: u32,
        (desc, keymap, networks): &DescriptorTemplateOut,
    ) -> io::Result<()> {
        // the amount of allocation here hurts me emotionally
        // yes, we can't just serialize desc to bytes 👁️👁️
        let desc_string = desc.to_string();
        let db_key = {
            let mut key = Vec::from(recover_at.to_be_bytes());
            // this will actually write the private key inside the descriptor so we hash it
            let mut hasher = <Sha256 as Digest>::new(); // this is to appease the analyzer
            hasher.update(desc_string.as_bytes());
            key.extend_from_slice(hasher.finalize().as_ref());
            key
        };

        let keymap_iter = keymap
            .iter()
            .map(|(pubk, privk)| [pubk.to_string(), privk.to_string()])
            .map(|[pubk, privk]| {
                (
                    (pubk.len() as u32).to_le_bytes(),
                    pubk,
                    (privk.len() as u32).to_le_bytes(),
                    privk,
                )
            });

        // descriptor length: u64 le
        // descriptor: string
        // keymap length in bytes: u64 le
        // keymap: [
        //  pubk_len: u32 le
        //  pubk
        //  privk_len: u32 le
        //  privk
        // ]
        // num networks: u8 le
        // networks: [
        //  network chain hash: 32 byte
        // ]
        let mut bytes = Vec::new();

        let desc_bytes = desc_string.as_bytes();
        bytes.extend_from_slice(&(desc_bytes.len() as u64).to_le_bytes());
        bytes.extend_from_slice(desc_bytes);

        let keymap_len = keymap_iter
            .clone()
            .map(|(pubk_len, pubk, privk_len, privk)| {
                pubk_len.len() + pubk.len() + privk_len.len() + privk.len()
            })
            .sum::<usize>();

        bytes.extend_from_slice(&(keymap_len as u64).to_le_bytes());

        for (pubk_len, pubk, privk_len, privk) in keymap_iter {
            bytes.extend_from_slice(&pubk_len);
            bytes.extend_from_slice(pubk.as_bytes());
            bytes.extend_from_slice(&privk_len);
            bytes.extend_from_slice(privk.as_bytes());
        }

        let networks = networks
            .iter()
            .map(|n| n.chain_hash().to_bytes())
            .collect::<Vec<_>>();
        let networks_len = networks.len() as u8;

        bytes.extend_from_slice(&networks_len.to_le_bytes());
        for net in networks {
            bytes.extend_from_slice(&net);
        }

        let mut nonce = Nonce::default();
        OsRng.fill_bytes(&mut nonce);

        // encrypted_bytes | tag (16 bytes) | nonce (12 bytes)
        self.cipher
            .encrypt_in_place(&nonce, &[], &mut bytes)
            .expect("encryption should succeed");

        bytes.extend_from_slice(nonce.as_ref());

        self.db.insert(db_key, bytes)?;
        self.db.flush_async().await?;
        Ok(())
    }

    pub async fn open(seed: &Seed, descriptor_db: &Path) -> io::Result<Self> {
        let key = seed.descriptor_recovery_key();
        let cipher = Aes256GcmSiv::new(&key.into());
        let db = sled::open(descriptor_db)?;
        let counter_tree = db.open_tree(b"reclaim_counter")?;
        Ok(Self {
            db,
            counter_tree,
            cipher,
        })
    }

    pub async fn read_descs(
        &mut self,
        height_range: impl RangeBounds<u32>,
    ) -> Result<
        Vec<(DescriptorRecoveryKey, DescriptorTemplateOut)>,
        OneOf<(
            InvalidDescriptor,
            InvalidNetwork,
            InvalidNetworksLen,
            InvalidPrivateKey,
            InvalidPublicKey,
            aes_gcm_siv::Error,
            io::Error,
            sled::Error,
            EntryTooShort,
        )>,
    > {
        let after_height = self
            .db
            .range(BigEndianRangeBounds::from_u32_range(height_range));
        let mut descs = vec![];
        for desc_entry in after_height {
            let (key, mut raw) = desc_entry.map_err(OneOf::new)?;
            let key = DescriptorRecoveryKey::decode(&key).unwrap();
            if raw.len() <= 12 + 16 {
                return Err(OneOf::new(EntryTooShort { length: raw.len() }));
            }
            let split_at = raw.len() - 12;
            let (rest, nonce) = raw.split_at_mut(split_at);
            let nonce = Nonce::from_slice(nonce);
            let (encrypted, tag) = rest.split_at_mut(rest.len() - 16);
            let tag = Tag::from_slice(tag);

            self.cipher
                .decrypt_in_place_detached(nonce, &[], encrypted, tag)
                .map_err(OneOf::new)?;

            let decrypted = encrypted;
            let mut cursor = Cursor::new(&decrypted);

            let desc_len = cursor.read_u64_le().await.map_err(OneOf::new)? as usize;
            let mut desc_bytes = vec![0u8; desc_len];
            Read::read_exact(&mut cursor, &mut desc_bytes).map_err(OneOf::new)?;
            let desc = String::from_utf8(desc_bytes)
                // oh yeah, nested terrors
                .map_err(|e| OneOf::new(InvalidDescriptor(OneOf::new(e))))?;
            let desc = Descriptor::<DescriptorPublicKey>::from_str(&desc)
                .map_err(|e| OneOf::new(InvalidDescriptor(OneOf::new(e))))?;

            let keymap_len = cursor.read_u64_le().await.map_err(OneOf::new)? as usize;

            let mut to_read = keymap_len;
            let mut keymap = BTreeMap::new();
            while to_read > 0 {
                let pubk_len = cursor.read_u32_le().await.map_err(OneOf::new)? as usize;
                to_read -= 4;

                let mut pubk_bytes = vec![0u8; pubk_len];
                Read::read_exact(&mut cursor, &mut pubk_bytes).map_err(OneOf::new)?;
                to_read -= pubk_len;

                let pubk = String::from_utf8(pubk_bytes)
                    .map_err(|e| OneOf::new(InvalidPublicKey(OneOf::new(e))))?;
                let pubk = DescriptorPublicKey::from_str(&pubk)
                    .map_err(|e| OneOf::new(InvalidPublicKey(OneOf::new(e))))?;

                let privk_len = cursor.read_u32_le().await.map_err(OneOf::new)? as usize;
                to_read -= 4;

                let mut privk_bytes = vec![0u8; privk_len];
                Read::read_exact(&mut cursor, &mut privk_bytes).map_err(OneOf::new)?;
                to_read -= privk_len;

                let privk = String::from_utf8(privk_bytes)
                    .map_err(|e| OneOf::new(InvalidPrivateKey(OneOf::new(e))))?;
                let privk = DescriptorSecretKey::from_str(&privk)
                    .map_err(|e| OneOf::new(InvalidPrivateKey(OneOf::new(e))))?;
                keymap.insert(pubk, privk);
            }

            let networks_len = cursor.read_u8().await.map_err(OneOf::new)?;
            let mut networks = HashSet::with_capacity(networks_len as usize);
            for _ in 0..networks_len {
                let mut chain_hash = [0u8; 32];
                Read::read_exact(&mut cursor, &mut chain_hash).map_err(OneOf::new)?;
                let network = Network::from_chain_hash(ChainHash::from(chain_hash))
                    .ok_or(OneOf::new(InvalidNetwork))?;
                networks.insert(network);
            }

            descs.push((key, (desc, keymap, networks)));
        }
        Ok(descs)
    }

    pub fn remove(&self, key: &DescriptorRecoveryKey) -> sled::Result<Option<sled::IVec>> {
        self.db.remove(key.encode())
    }
}

#[derive(Debug)]
#[expect(unused, reason = "Error type for recovery operations")]
pub struct EntryTooShort {
    length: usize,
}

#[derive(Debug)]
#[expect(unused, reason = "Error type for invalid descriptor recovery")]
pub struct InvalidDescriptor(OneOf<(FromUtf8Error, miniscript::Error)>);

#[derive(Debug)]
pub struct InvalidNetwork;

#[derive(Debug)]
pub struct InvalidNetworksLen;

#[derive(Debug)]
#[expect(unused, reason = "Error type for invalid private key recovery")]
pub struct InvalidPrivateKey(OneOf<(FromUtf8Error, DescriptorKeyParseError)>);

#[derive(Debug)]
#[expect(unused, reason = "Error type for invalid public key recovery")]
pub struct InvalidPublicKey(OneOf<(FromUtf8Error, DescriptorKeyParseError)>);

#[derive(Debug)]
pub struct DescriptorRecoveryKey {
    pub recover_at: u32,
    pub desc_string_hash: [u8; Self::DESC_STRING_HASH_SIZE],
}

impl DescriptorRecoveryKey {
    const ENCODED_SIZE: usize = Self::DESC_STRING_HASH_SIZE + Self::RECOVER_AT_SIZE;
    const DESC_STRING_HASH_SIZE: usize = 32;
    const RECOVER_AT_SIZE: usize = (u32::BITS / 8) as usize;

    pub fn new(recover_at: u32, desc: &Descriptor<DescriptorPublicKey>) -> Self {
        let mut hasher = <Sha256 as Digest>::new(); // this is to appease the analyzer
        hasher.update(desc.to_string().as_bytes());
        Self {
            recover_at,
            desc_string_hash: hasher.finalize().into(),
        }
    }

    pub fn encode(&self) -> [u8; Self::ENCODED_SIZE] {
        make_buf! {
            (&self.recover_at.to_be_bytes(), DescriptorRecoveryKey::RECOVER_AT_SIZE),
            (&self.desc_string_hash, DescriptorRecoveryKey::DESC_STRING_HASH_SIZE)
        }
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::ENCODED_SIZE {
            return None;
        }

        let recover_at = u32::from_be_bytes(unsafe {
            *(bytes[..Self::RECOVER_AT_SIZE].as_ptr() as *const [_; Self::RECOVER_AT_SIZE])
        });
        let mut desc_string_hash = [0u8; Self::DESC_STRING_HASH_SIZE];
        desc_string_hash.copy_from_slice(&bytes[Self::RECOVER_AT_SIZE..]);

        Some(Self {
            recover_at,
            desc_string_hash,
        })
    }
}

/// A helper so that we can pass ranges of block heights as u32s when reading descriptors,
/// but the database actually needs to do the range via the big endian representation.
struct BigEndianRangeBounds {
    start: Bound<[u8; 4]>,
    end: Bound<[u8; 4]>,
}

impl BigEndianRangeBounds {
    fn from_u32_range<R: RangeBounds<u32>>(range: R) -> Self {
        let start = match range.start_bound() {
            Bound::Included(&n) => Bound::Included(n.to_be_bytes()),
            Bound::Excluded(&n) => Bound::Excluded(n.to_be_bytes()),
            Bound::Unbounded => Bound::Unbounded,
        };

        let end = match range.end_bound() {
            Bound::Included(&n) => Bound::Included(n.to_be_bytes()),
            Bound::Excluded(&n) => Bound::Excluded(n.to_be_bytes()),
            Bound::Unbounded => Bound::Unbounded,
        };

        Self { start, end }
    }
}

impl RangeBounds<[u8; 4]> for BigEndianRangeBounds {
    fn start_bound(&self) -> Bound<&[u8; 4]> {
        match &self.start {
            Bound::Included(arr) => Bound::Included(arr),
            Bound::Excluded(arr) => Bound::Excluded(arr),
            Bound::Unbounded => Bound::Unbounded,
        }
    }

    fn end_bound(&self) -> Bound<&[u8; 4]> {
        match &self.end {
            Bound::Included(arr) => Bound::Included(arr),
            Bound::Excluded(arr) => Bound::Excluded(arr),
            Bound::Unbounded => Bound::Unbounded,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf, process};

    use super::*;

    fn temp_db_path() -> PathBuf {
        env::temp_dir().join(format!(
            "alpen-cli-recovery-test-{}-{}",
            process::id(),
            OsRng.next_u64()
        ))
    }

    #[tokio::test]
    // The counter must never repeat across a process restart, since a reused counter would
    // derive the same reclaim key for two different deposit requests.
    async fn next_reclaim_counter_survives_reopen() {
        let seed = Seed::gen(&mut OsRng);
        let path = temp_db_path();

        let recovery = DescriptorRecovery::open(&seed, &path)
            .await
            .expect("should open descriptor db");
        recovery.ensure_reclaim_counter_at_least(0).await.unwrap();
        let first = recovery.next_reclaim_counter().await.unwrap();
        drop(recovery);

        let second = DescriptorRecovery::open(&seed, &path)
            .await
            .expect("should reopen descriptor db")
            .next_reclaim_counter()
            .await
            .unwrap();

        assert_ne!(
            first, second,
            "reopening must not hand out a counter already used"
        );

        fs::remove_dir_all(&path).ok();
    }

    #[tokio::test]
    async fn missing_reclaim_counter_must_be_reconstructed_before_allocation() {
        let seed = Seed::gen(&mut OsRng);
        let path = temp_db_path();
        let recovery = DescriptorRecovery::open(&seed, &path)
            .await
            .expect("should open descriptor db");

        let error = recovery.next_reclaim_counter().await.unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(recovery.current_reclaim_counter().unwrap(), None);
        fs::remove_dir_all(&path).ok();
    }

    #[tokio::test]
    async fn reconstructed_reclaim_counter_never_rolls_back() {
        let seed = Seed::gen(&mut OsRng);
        let path = temp_db_path();
        let recovery = DescriptorRecovery::open(&seed, &path)
            .await
            .expect("should open descriptor db");

        assert_eq!(
            recovery.ensure_reclaim_counter_at_least(7).await.unwrap(),
            7
        );
        assert_eq!(
            recovery.ensure_reclaim_counter_at_least(3).await.unwrap(),
            7
        );
        assert_eq!(recovery.next_reclaim_counter().await.unwrap(), 8);

        fs::remove_dir_all(&path).ok();
    }

    #[tokio::test]
    // Regression test: before the counter moved into its own tree, `read_descs`'s range scan
    // saw the 19-byte counter key alongside real (always 36-byte) descriptor keys, and
    // `DescriptorRecoveryKey::decode(&key).unwrap()` panicked on it. `alpen debug recovery`
    // calls `read_descs(..)` on any populated database, so this used to panic on every deposit.
    async fn read_descs_ignores_reclaim_counter_state() {
        let seed = Seed::gen(&mut OsRng);
        let path = temp_db_path();

        let mut recovery = DescriptorRecovery::open(&seed, &path)
            .await
            .expect("should open descriptor db");
        recovery.ensure_reclaim_counter_at_least(0).await.unwrap();
        recovery.next_reclaim_counter().await.unwrap();

        let descs = recovery
            .read_descs(..)
            .await
            .expect("should not panic or error on the counter's tree");
        assert!(descs.is_empty());

        fs::remove_dir_all(&path).ok();
    }
}
