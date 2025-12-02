//! Codec helper functions for encoding/decoding with length prefixes.
//!
//! This module provides utility functions for encoding and decoding data with length prefixes,
//! useful for variable-length fields in the EVM execution environment types.
//!
//! Also includes custom Codec implementations for external types (RSP and Reth) that don't
//! have native Codec support, avoiding the use of unstable bincode serialization.

use std::collections::BTreeMap;

use reth_trie::HashedPostState;
use revm_primitives::{B256, HashMap as RevmHashMap};
use rsp_mpt::EthereumState;
use strata_codec::{Codec, CodecError, Varint};

/// Encodes an RLP-encodable item with a varint length prefix.
///
/// This encodes the item using RLP, then writes a varint length prefix followed by the RLP bytes.
/// Varints are more space-efficient for small lengths.
pub(crate) fn encode_rlp_with_length<T: alloy_rlp::Encodable>(
    item: &T,
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    let rlp_encoded = alloy_rlp::encode(item);
    let len = Varint::new(rlp_encoded.len() as u32)
        .ok_or(CodecError::MalformedField("length too large for varint"))?;
    len.encode(enc)?;
    enc.write_buf(&rlp_encoded)?;
    Ok(())
}

/// Decodes an RLP-decodable item with a varint length prefix.
///
/// This reads a varint length prefix, then reads that many bytes and decodes them using RLP.
pub(crate) fn decode_rlp_with_length<T: alloy_rlp::Decodable>(
    dec: &mut impl strata_codec::Decoder,
) -> Result<T, CodecError> {
    let len_varint = Varint::decode(dec)?;
    let len = len_varint.inner() as usize;
    let mut buf = vec![0u8; len];
    dec.read_buf(&mut buf)?;

    alloy_rlp::Decodable::decode(&mut &buf[..])
        .map_err(|_| CodecError::MalformedField("RLP decode failed"))
}

/// Encodes raw bytes with a varint length prefix.
///
/// This writes a varint length prefix followed by the raw bytes.
/// Varints are more space-efficient for small lengths.
pub(crate) fn encode_bytes_with_length(
    bytes: &[u8],
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    let len = Varint::new(bytes.len() as u32)
        .ok_or(CodecError::MalformedField("length too large for varint"))?;
    len.encode(enc)?;
    enc.write_buf(bytes)?;
    Ok(())
}

/// Decodes raw bytes with a varint length prefix.
///
/// This reads a varint length prefix, then reads that many bytes and returns them as a Vec<u8>.
pub(crate) fn decode_bytes_with_length(
    dec: &mut impl strata_codec::Decoder,
) -> Result<Vec<u8>, CodecError> {
    let len_varint = Varint::decode(dec)?;
    let len = len_varint.inner() as usize;
    let mut bytes = vec![0u8; len];
    dec.read_buf(&mut bytes)?;
    Ok(bytes)
}

// ============================================================================
// Custom deterministic encoding for external types (RSP, Reth)
// ============================================================================

/// Encodes EthereumState deterministically (sorts HashMap entries).
///
/// Uses RLP encoding for MptNode (which is deterministic) and sorts the
/// storage_tries HashMap by key to ensure deterministic iteration order.
pub(crate) fn encode_ethereum_state(
    state: &EthereumState,
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    // Encode state_trie using RLP (MptNode implements alloy_rlp::Encodable)
    encode_rlp_with_length(&state.state_trie, enc)?;

    // Sort storage_tries by key for deterministic encoding
    let sorted_storage: BTreeMap<_, _> = state.storage_tries.iter().collect();
    (sorted_storage.len() as u32).encode(enc)?;

    for (address_hash, storage_trie) in sorted_storage {
        enc.write_buf(address_hash.as_slice())?;
        // Encode each storage trie using RLP
        encode_rlp_with_length(storage_trie, enc)?;
    }

    Ok(())
}

/// Decodes EthereumState.
pub(crate) fn decode_ethereum_state(
    dec: &mut impl strata_codec::Decoder,
) -> Result<EthereumState, CodecError> {
    // Decode state_trie using rlp (MptNode implements rlp::Decodable)
    let state_trie_bytes = decode_bytes_with_length(dec)?;
    let state_trie = rlp::decode(&state_trie_bytes)
        .map_err(|_| CodecError::MalformedField("state_trie RLP decode failed"))?;

    // Decode storage_tries
    let storage_count = u32::decode(dec)? as usize;
    let mut storage_tries =
        RevmHashMap::with_capacity_and_hasher(storage_count, Default::default());

    for _ in 0..storage_count {
        let mut address_hash_bytes = [0u8; 32];
        dec.read_buf(&mut address_hash_bytes)?;
        let address_hash = B256::from(address_hash_bytes);

        // Decode storage trie using rlp (MptNode implements rlp::Decodable)
        let storage_trie_bytes = decode_bytes_with_length(dec)?;
        let storage_trie = rlp::decode(&storage_trie_bytes)
            .map_err(|_| CodecError::MalformedField("storage_trie RLP decode failed"))?;
        storage_tries.insert(address_hash, storage_trie);
    }

    // Construct EthereumState directly from public fields
    Ok(EthereumState {
        state_trie,
        storage_tries,
    })
}

/// Encodes HashedPostState deterministically (sorts HashMap entries).
///
/// HashedPostState contains two HashMaps (accounts and storages) which we sort
/// by key before encoding to ensure deterministic iteration order.
pub(crate) fn encode_hashed_post_state(
    state: &HashedPostState,
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    // Sort accounts by key for deterministic encoding
    let sorted_accounts: BTreeMap<_, _> = state.accounts.iter().collect();
    (sorted_accounts.len() as u32).encode(enc)?;

    for (address_hash, account_opt) in sorted_accounts {
        enc.write_buf(address_hash.as_slice())?;

        // Encode Option<Account>
        match account_opt {
            Some(account) => {
                true.encode(enc)?;
                // Encode Account fields
                account.nonce.encode(enc)?;
                enc.write_buf(account.balance.as_le_slice())?;
                match &account.bytecode_hash {
                    Some(hash) => {
                        true.encode(enc)?;
                        enc.write_buf(hash.as_slice())?;
                    }
                    None => {
                        false.encode(enc)?;
                    }
                }
            }
            None => {
                false.encode(enc)?;
            }
        }
    }

    // Sort storages by key for deterministic encoding
    let sorted_storages: BTreeMap<_, _> = state.storages.iter().collect();
    (sorted_storages.len() as u32).encode(enc)?;

    for (address_hash, hashed_storage) in sorted_storages {
        enc.write_buf(address_hash.as_slice())?;

        // Encode HashedStorage
        hashed_storage.wiped.encode(enc)?;

        // Sort storage slots by key
        let sorted_storage_slots: BTreeMap<_, _> = hashed_storage.storage.iter().collect();
        (sorted_storage_slots.len() as u32).encode(enc)?;

        for (slot_hash, value) in sorted_storage_slots {
            enc.write_buf(slot_hash.as_slice())?;
            enc.write_buf(value.as_le_slice())?;
        }
    }

    Ok(())
}

/// Decodes HashedPostState.
pub(crate) fn decode_hashed_post_state(
    dec: &mut impl strata_codec::Decoder,
) -> Result<HashedPostState, CodecError> {
    use reth_primitives::Account;
    use reth_trie::HashedStorage;
    use revm_primitives::U256;

    // Start with empty HashedPostState
    let mut state = HashedPostState::default();

    // Decode accounts
    let accounts_count = u32::decode(dec)? as usize;

    for _ in 0..accounts_count {
        let mut address_hash_bytes = [0u8; 32];
        dec.read_buf(&mut address_hash_bytes)?;
        let address_hash = B256::from(address_hash_bytes);

        let has_account = bool::decode(dec)?;
        let account_opt = if has_account {
            let nonce = u64::decode(dec)?;

            let mut balance_bytes = [0u8; 32];
            dec.read_buf(&mut balance_bytes)?;
            let balance = U256::from_le_slice(&balance_bytes);

            let has_bytecode = bool::decode(dec)?;
            let bytecode_hash = if has_bytecode {
                let mut hash_bytes = [0u8; 32];
                dec.read_buf(&mut hash_bytes)?;
                Some(B256::from(hash_bytes))
            } else {
                None
            };

            Some(Account {
                nonce,
                balance,
                bytecode_hash,
            })
        } else {
            None
        };

        state.accounts.insert(address_hash, account_opt);
    }

    // Decode storages
    let storages_count = u32::decode(dec)? as usize;

    for _ in 0..storages_count {
        let mut address_hash_bytes = [0u8; 32];
        dec.read_buf(&mut address_hash_bytes)?;
        let address_hash = B256::from(address_hash_bytes);

        let wiped = bool::decode(dec)?;

        let storage_slots_count = u32::decode(dec)? as usize;
        let mut hashed_storage = HashedStorage::default();

        for _ in 0..storage_slots_count {
            let mut slot_hash_bytes = [0u8; 32];
            dec.read_buf(&mut slot_hash_bytes)?;
            let slot_hash = B256::from(slot_hash_bytes);

            let mut value_bytes = [0u8; 32];
            dec.read_buf(&mut value_bytes)?;
            let value = U256::from_le_slice(&value_bytes);

            hashed_storage.storage.insert(slot_hash, value);
        }

        hashed_storage.wiped = wiped;
        state.storages.insert(address_hash, hashed_storage);
    }

    Ok(state)
}
