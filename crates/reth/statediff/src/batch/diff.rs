//! Main batch state diff type for DA encoding.

use std::collections::BTreeMap;

use alloy_primitives::Bytes;
use revm_primitives::{Address, B256};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{decode_map_with, encode_map_with};

use super::{AccountChange, StorageDiff};
use crate::codec::{CodecAddress, CodecB256};

/// Complete state diff for a batch, optimized for DA encoding.
///
/// This is the type that gets posted to the DA layer. It represents
/// the net change over a range of blocks, with reverts already filtered out.
/// The current encoding is pinned by golden fixtures under `testdata/` to catch
/// accidental wire-format drift until a deliberate compatibility migration exists.
#[derive(Clone, Debug, Default)]
pub struct BatchStateDiff {
    /// Account changes, sorted by address for deterministic encoding.
    pub accounts: BTreeMap<Address, AccountChange>,
    /// Storage slot changes per account, sorted by address.
    pub storage: BTreeMap<Address, StorageDiff>,
    /// Deployed contract bytecodes keyed by code hash (deduplicated).
    /// Full bytecode is included for DA reconstruction without DB access.
    pub deployed_bytecodes: BTreeMap<B256, Bytes>,
}

impl BatchStateDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the diff is empty.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.storage.is_empty() && self.deployed_bytecodes.is_empty()
    }
}

/// Wrapper for Bytes that implements Codec with length-prefixed encoding.
#[derive(Clone, Debug)]
struct CodecBytes(Bytes);

impl Codec for CodecBytes {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode length as u32, then raw bytes
        (self.0.len() as u32).encode(enc)?;
        enc.write_buf(&self.0)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u32::decode(dec)? as usize;
        let mut buf = vec![0u8; len];
        dec.read_buf(&mut buf)?;
        Ok(Self(Bytes::from(buf)))
    }
}

impl Codec for BatchStateDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        encode_map_with(&self.accounts, enc, |a| CodecAddress(*a), Clone::clone)?;
        encode_map_with(&self.storage, enc, |a| CodecAddress(*a), Clone::clone)?;
        // Encode bytecodes as map: hash -> bytes
        encode_map_with(
            &self.deployed_bytecodes,
            enc,
            |h| CodecB256(*h),
            |b| CodecBytes(b.clone()),
        )?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let accounts = decode_map_with(dec, |k: CodecAddress| k.0, |v| v)?;
        let storage = decode_map_with(dec, |k: CodecAddress| k.0, |v| v)?;
        let deployed_bytecodes = decode_map_with(dec, |k: CodecB256| k.0, |v: CodecBytes| v.0)?;

        Ok(Self {
            accounts,
            storage,
            deployed_bytecodes,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_da_framework::SignedVarInt;

    use super::*;
    use crate::{
        batch::AccountDiff,
        test_utils::{
            account_change, block_diff, deployed_bytecode, hash, snapshot, storage_change,
        },
        BatchBuilder,
    };

    #[test]
    fn test_batch_state_diff_roundtrip() {
        let mut diff = BatchStateDiff::new();

        // Add account change
        diff.accounts.insert(
            Address::from([0x11u8; 20]),
            AccountChange::Created(AccountDiff::new_created(
                U256::from(1000),
                1,
                B256::from([0x22u8; 32]),
            )),
        );

        // Add storage change
        let mut storage = StorageDiff::new();
        storage.set_slot(U256::from(1), U256::from(100));
        diff.storage.insert(Address::from([0x11u8; 20]), storage);

        // Add deployed bytecode
        let bytecode = Bytes::from_static(&[0x60, 0x80, 0x60, 0x40, 0x52]); // Sample EVM bytecode
        diff.deployed_bytecodes
            .insert(B256::from([0x33u8; 32]), bytecode.clone());

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: BatchStateDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.accounts.len(), 1);
        assert_eq!(decoded.storage.len(), 1);
        assert_eq!(decoded.deployed_bytecodes.len(), 1);
        assert_eq!(
            decoded
                .deployed_bytecodes
                .get(&B256::from([0x33u8; 32]))
                .unwrap(),
            &bytecode
        );
    }

    #[test]
    fn test_empty_diff_size() {
        let diff = BatchStateDiff::new();
        let encoded = encode_to_vec(&diff).unwrap();
        // Should be minimal: 3 u32 counts (0, 0, 0) = 12 bytes
        assert!(encoded.len() <= 12);
    }

    #[test]
    fn test_bytecode_encoding_size() {
        let mut diff = BatchStateDiff::new();

        // Add a realistic contract bytecode (~1KB)
        let bytecode = Bytes::from(vec![0x60u8; 1024]);
        diff.deployed_bytecodes
            .insert(B256::from([0x11u8; 32]), bytecode);

        let encoded = encode_to_vec(&diff).unwrap();
        // Should include: 3 map counts + 32 byte hash + 4 byte length + 1024 bytes
        // Plus some overhead for map encoding
        assert!(encoded.len() > 1024);
        assert!(encoded.len() < 1100); // Not too much overhead
    }

    #[test]
    fn test_batch_state_diff_rejects_trailing_bytes() {
        let mut encoded = encode_to_vec(&BatchStateDiff::new()).unwrap();
        encoded.push(0xff);
        assert!(decode_buf_exact::<BatchStateDiff>(&encoded).is_err());
    }

    #[test]
    fn test_batch_state_diff_rejects_truncated_bytecode_payload() {
        let mut diff = BatchStateDiff::new();
        diff.deployed_bytecodes
            .insert(B256::from([0x44u8; 32]), Bytes::from_static(&[1, 2, 3, 4]));

        let mut encoded = encode_to_vec(&diff).unwrap();
        encoded.pop();
        assert!(decode_buf_exact::<BatchStateDiff>(&encoded).is_err());
    }

    #[test]
    fn test_batch_state_diff_rejects_bytecode_length_overrun() {
        let mut diff = BatchStateDiff::new();
        diff.deployed_bytecodes.insert(
            B256::from([0x55u8; 32]),
            Bytes::from_static(&[0xaa, 0xbb, 0xcc]),
        );

        let mut encoded = encode_to_vec(&diff).unwrap();
        // Layout here is: 3 x 4-byte map counts, then the 32-byte bytecode hash,
        // then the bytecode length field we intentionally corrupt.
        let offset = 12 + 32;
        let oversized_len = encode_to_vec(&(10u32)).unwrap();
        encoded[offset..offset + 4].copy_from_slice(&oversized_len);

        assert!(decode_buf_exact::<BatchStateDiff>(&encoded).is_err());
    }

    fn fixture_single_account_create() -> BatchStateDiff {
        let mut block = block_diff();
        account_change(
            &mut block,
            Address::from([0x11u8; 20]),
            None,
            Some(snapshot(1000, 1, hash(0x21))),
        );
        BatchStateDiff::from(block)
    }

    fn fixture_storage_only_update() -> BatchStateDiff {
        let address = Address::from([0x12u8; 20]);
        let mut block = block_diff();
        storage_change(
            &mut block,
            address,
            U256::from(1),
            U256::from(5),
            U256::from(7),
        );
        storage_change(
            &mut block,
            address,
            U256::from(2),
            U256::ZERO,
            U256::from(9),
        );
        BatchStateDiff::from(block)
    }

    fn fixture_selfdestruct_recreate() -> BatchStateDiff {
        let address = Address::from([0x13u8; 20]);

        let mut block_one = block_diff();
        account_change(
            &mut block_one,
            address,
            Some(snapshot(900, 7, hash(0x22))),
            None,
        );
        storage_change(
            &mut block_one,
            address,
            U256::from(1),
            U256::from(33),
            U256::ZERO,
        );

        let mut block_two = block_diff();
        account_change(
            &mut block_two,
            address,
            None,
            Some(snapshot(55, 1, hash(0x23))),
        );
        storage_change(
            &mut block_two,
            address,
            U256::from(2),
            U256::ZERO,
            U256::from(44),
        );

        let mut builder = BatchBuilder::new();
        builder.apply_block(&block_one);
        builder.apply_block(&block_two);
        builder.build()
    }

    fn fixture_code_hash_and_bytecode_update() -> BatchStateDiff {
        let address = Address::from([0x14u8; 20]);
        let old_hash = hash(0x24);
        let new_hash = hash(0x25);
        let mut block = block_diff();
        account_change(
            &mut block,
            address,
            Some(snapshot(500, 8, old_hash)),
            Some(snapshot(500, 8, new_hash)),
        );
        storage_change(
            &mut block,
            address,
            U256::from(1),
            U256::from(1),
            U256::from(3),
        );
        deployed_bytecode(
            &mut block,
            new_hash,
            Bytes::from_static(&[0x60, 0x80, 0x60, 0x40, 0x52]),
        );
        BatchStateDiff::from(block)
    }

    fn fixture_cases() -> [(&'static str, BatchStateDiff); 5] {
        [
            ("empty_batch", BatchStateDiff::new()),
            ("single_account_create", fixture_single_account_create()),
            ("storage_only_update", fixture_storage_only_update()),
            ("selfdestruct_recreate", fixture_selfdestruct_recreate()),
            (
                "code_hash_and_bytecode_update",
                fixture_code_hash_and_bytecode_update(),
            ),
        ]
    }

    fn assert_account_diff_semantics(actual: &AccountDiff, expected: &AccountDiff) {
        assert_eq!(actual.balance.diff(), expected.balance.diff());
        assert_eq!(
            actual.nonce.diff().and_then(SignedVarInt::to_i64),
            expected.nonce.diff().and_then(SignedVarInt::to_i64)
        );
        assert_eq!(
            actual.code_hash.new_value().map(|value| value.0),
            expected.code_hash.new_value().map(|value| value.0)
        );
    }

    fn assert_account_change_semantics(actual: &AccountChange, expected: &AccountChange) {
        match (actual, expected) {
            (AccountChange::Created(actual), AccountChange::Created(expected))
            | (AccountChange::Updated(actual), AccountChange::Updated(expected)) => {
                assert_account_diff_semantics(actual, expected);
            }
            (AccountChange::Deleted, AccountChange::Deleted) => {}
            _ => panic!("account change variant mismatch"),
        }
    }

    fn assert_batch_diff_semantics(actual: &BatchStateDiff, expected: &BatchStateDiff) {
        assert_eq!(actual.accounts.len(), expected.accounts.len());
        assert_eq!(actual.storage, expected.storage);
        assert_eq!(actual.deployed_bytecodes, expected.deployed_bytecodes);

        for (address, expected_change) in &expected.accounts {
            let actual_change = actual.accounts.get(address).unwrap();
            assert_account_change_semantics(actual_change, expected_change);
        }
    }

    fn fixture_bytes(name: &str) -> &'static [u8] {
        match name {
            "empty_batch" => include_bytes!("../../testdata/empty_batch.bin"),
            "single_account_create" => include_bytes!("../../testdata/single_account_create.bin"),
            "storage_only_update" => include_bytes!("../../testdata/storage_only_update.bin"),
            "selfdestruct_recreate" => include_bytes!("../../testdata/selfdestruct_recreate.bin"),
            "code_hash_and_bytecode_update" => {
                include_bytes!("../../testdata/code_hash_and_bytecode_update.bin")
            }
            _ => panic!("unknown fixture {name}"),
        }
    }

    #[test]
    fn test_batch_state_diff_golden_fixtures() {
        for (name, expected_diff) in fixture_cases() {
            let fixture = fixture_bytes(name);
            let encoded = encode_to_vec(&expected_diff).unwrap();
            assert_eq!(
                encoded.as_slice(),
                fixture,
                "fixture bytes changed for {name}"
            );

            let decoded: BatchStateDiff = decode_buf_exact(fixture).unwrap();
            assert_batch_diff_semantics(&decoded, &expected_diff);
        }
    }
}
