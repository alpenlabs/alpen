//! Bitcoin Merkle / header-chain verification primitives for the EE
//! outer (acct) proof.
//!
//! Used by the guest to verify, for each reveal tx in the DA witness:
//!
//! 1. The reveal's wtxid is included in some Bitcoin block — proven via a wtxid Merkle path against
//!    the block's witness root, which the coinbase OP_RETURN commits to.
//! 2. That block's header chains up to the public `l1_block_hash` — proven via a sequence of
//!    consensus-encoded headers each linking by `prev_blockhash`.
//!
//! No PoW is checked here. The OL re-runs canonicality + sufficient
//! depth against its own L1 Header MMR when processing the resulting
//! `EEUpdate`; in-proof we only attest "included under this hash's
//! ancestry."

use bitcoin::{
    Transaction, block::Header as BitcoinHeader, consensus::deserialize, hashes::Hash as _,
};
use strata_crypto::hash::sha256d;
use strata_identifiers::Buf32;

/// Errors raised by Bitcoin DA inclusion checks.
#[derive(Debug, thiserror::Error)]
pub enum DaInclusionError {
    #[error("malformed Bitcoin header: {0}")]
    MalformedHeader(String),
    #[error("malformed coinbase tx: {0}")]
    MalformedCoinbase(String),
    #[error("header chain link mismatch at index {idx}")]
    HeaderChainBreak { idx: usize },
    #[error("header chain does not end at expected tip")]
    TipMismatch,
    #[error("coinbase Merkle proof root mismatch")]
    CoinbaseMerkleMismatch,
    #[error("coinbase has no BIP-141 witness commitment")]
    MissingWitnessCommitment,
    #[error("coinbase has malformed witness reserved value")]
    MalformedWitnessReserved,
    #[error("witness commitment does not match reconstructed witness root")]
    WitnessCommitmentMismatch,
}

/// Walks `start_header` through `chain_to_tip`, verifying that each
/// successor's `prev_blockhash` equals its predecessor's hash and that
/// the final header's hash equals `expected_tip_hash`.
///
/// Headers are passed in consensus-encoded 80-byte form. The chain is
/// taken as any iterator of `&[u8]` so callers can hand in either an
/// owned `&[Vec<u8>]` (host tests) or rkyv archived bytes (guest)
/// without intermediate allocation.
///
/// Returns the hash of `start_header` so callers can use it as the
/// block hash for further inclusion checks.
pub fn verify_header_chain<'a, I>(
    start_header: &[u8],
    chain_to_tip: I,
    expected_tip_hash: [u8; 32],
) -> Result<[u8; 32], DaInclusionError>
where
    I: IntoIterator<Item = &'a [u8]>,
{
    // Parse start header for early malformed-header detection.
    parse_header(start_header)?;
    let start_hash = block_hash(start_header);
    let mut prev_hash = start_hash;
    let mut walked_any = false;

    for (idx, raw) in chain_to_tip.into_iter().enumerate() {
        walked_any = true;
        let next = parse_header(raw)?;
        if next.prev_blockhash.to_byte_array() != prev_hash {
            return Err(DaInclusionError::HeaderChainBreak { idx });
        }
        prev_hash = block_hash(raw);
    }

    // chain empty ⇒ start_hash must equal tip; otherwise the last hash.
    let final_hash = if walked_any { prev_hash } else { start_hash };
    if final_hash != expected_tip_hash {
        return Err(DaInclusionError::TipMismatch);
    }
    Ok(start_hash)
}

/// Walks a Bitcoin Merkle proof from `leaf` upward. `siblings[i]`
/// concatenates left-or-right at level `i` according to bit `i` of
/// `position` (0 = current is left, 1 = current is right).
pub fn compute_btc_merkle_root(leaf: [u8; 32], siblings: &[[u8; 32]], position: u32) -> [u8; 32] {
    let mut cur = leaf;
    let mut pos = position;
    for sibling in siblings {
        let mut buf = [0u8; 64];
        if pos & 1 == 0 {
            buf[..32].copy_from_slice(&cur);
            buf[32..].copy_from_slice(sibling);
        } else {
            buf[..32].copy_from_slice(sibling);
            buf[32..].copy_from_slice(&cur);
        }
        cur = sha256d_buf32(&buf);
        pos >>= 1;
    }
    cur
}

/// Verifies that `coinbase_txid` is included under `header_merkle_root`
/// at the position implied by `siblings` + `position`.
pub fn verify_coinbase_inclusion(
    coinbase_txid: [u8; 32],
    siblings: &[[u8; 32]],
    position: u32,
    header_merkle_root: [u8; 32],
) -> Result<(), DaInclusionError> {
    let computed = compute_btc_merkle_root(coinbase_txid, siblings, position);
    if computed != header_merkle_root {
        return Err(DaInclusionError::CoinbaseMerkleMismatch);
    }
    Ok(())
}

/// Verifies `wtxid` is in the witness Merkle tree whose commitment is
/// embedded in the coinbase OP_RETURN.
///
/// Per BIP-141: `commitment = sha256d(witness_root || witness_reserved_value)`,
/// where `witness_reserved_value` lives in the coinbase's first input
/// witness as a single 32-byte stack element, and `witness_root` is the
/// Merkle root of all wtxids (with the coinbase's own wtxid replaced by
/// 32 zero bytes). The proof reconstructs `witness_root` from `wtxid`
/// + `siblings` + `position`, then checks the commitment.
pub fn verify_wtxid_inclusion(
    wtxid: [u8; 32],
    siblings: &[[u8; 32]],
    position: u32,
    coinbase_raw: &[u8],
) -> Result<(), DaInclusionError> {
    let coinbase = parse_coinbase(coinbase_raw)?;
    let commitment =
        extract_witness_commitment(&coinbase).ok_or(DaInclusionError::MissingWitnessCommitment)?;
    let reserved = extract_witness_reserved_value(&coinbase)?;
    let claimed_root = compute_btc_merkle_root(wtxid, siblings, position);

    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&claimed_root);
    buf[32..].copy_from_slice(&reserved);
    let computed_commitment = sha256d_buf32(&buf);
    if computed_commitment != commitment {
        return Err(DaInclusionError::WitnessCommitmentMismatch);
    }
    Ok(())
}

/// Returns the BIP-141 witness commitment (32 bytes) from a coinbase
/// transaction's last OP_RETURN output that begins with the magic
/// `[0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed]` prefix, or `None` if absent.
fn extract_witness_commitment(coinbase: &Transaction) -> Option<[u8; 32]> {
    const MAGIC: [u8; 6] = [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];
    let pos = coinbase
        .output
        .iter()
        .rposition(|o| o.script_pubkey.len() >= 38 && o.script_pubkey.as_bytes()[0..6] == MAGIC)?;
    let bytes = &coinbase.output[pos].script_pubkey.as_bytes()[6..38];
    Some(bytes.try_into().expect("38-6=32"))
}

/// Returns the 32-byte witness reserved value from a coinbase's first
/// input witness (must be exactly one 32-byte stack element per BIP-141).
fn extract_witness_reserved_value(coinbase: &Transaction) -> Result<[u8; 32], DaInclusionError> {
    let input = coinbase
        .input
        .first()
        .ok_or(DaInclusionError::MalformedWitnessReserved)?;
    let items: Vec<&[u8]> = input.witness.iter().collect();
    if items.len() != 1 || items[0].len() != 32 {
        return Err(DaInclusionError::MalformedWitnessReserved);
    }
    Ok(items[0].try_into().expect("len checked"))
}

fn parse_header(raw: &[u8]) -> Result<BitcoinHeader, DaInclusionError> {
    deserialize(raw).map_err(|e| DaInclusionError::MalformedHeader(e.to_string()))
}

fn parse_coinbase(raw: &[u8]) -> Result<Transaction, DaInclusionError> {
    deserialize(raw).map_err(|e| DaInclusionError::MalformedCoinbase(e.to_string()))
}

fn block_hash(header_raw: &[u8]) -> [u8; 32] {
    sha256d_buf32(header_raw)
}

fn sha256d_buf32(data: &[u8]) -> [u8; 32] {
    let buf: Buf32 = sha256d(data);
    *<Buf32 as AsRef<[u8; 32]>>::as_ref(&buf)
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
        absolute::LockTime, consensus::serialize, opcodes::all::OP_RETURN, script::Builder,
        transaction::Version,
    };

    use super::*;

    fn dsha(data: &[u8]) -> [u8; 32] {
        sha256d_buf32(data)
    }

    /// Builds a coinbase tx with a single 32-byte witness reserved value
    /// and an OP_RETURN output committing to `witness_root`.
    fn make_coinbase(witness_root: [u8; 32], reserved: [u8; 32]) -> Transaction {
        // commitment = sha256d(witness_root || reserved)
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&witness_root);
        buf[32..].copy_from_slice(&reserved);
        let commitment = dsha(&buf);

        // OP_RETURN OP_PUSHBYTES_36 <aa21a9ed> <commitment>
        let mut script_bytes = vec![0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];
        script_bytes.extend_from_slice(&commitment);
        let commitment_script = ScriptBuf::from_bytes(script_bytes);

        // Filler dust output to exercise rposition logic.
        let dust_script = Builder::new().push_opcode(OP_RETURN).into_script();

        let mut witness = Witness::new();
        witness.push(reserved);

        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness,
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(1234),
                    script_pubkey: dust_script,
                },
                TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: commitment_script,
                },
            ],
        }
    }

    /// Builds a fake 80-byte Bitcoin header with a given prev_blockhash
    /// and merkle_root; other fields are arbitrary fixed values.
    fn make_header(prev: [u8; 32], merkle_root: [u8; 32]) -> Vec<u8> {
        let mut h = Vec::with_capacity(80);
        h.extend_from_slice(&1u32.to_le_bytes()); // version
        h.extend_from_slice(&prev);
        h.extend_from_slice(&merkle_root);
        h.extend_from_slice(&1_700_000_000u32.to_le_bytes()); // time
        h.extend_from_slice(&0x1d00ffffu32.to_le_bytes()); // bits
        h.extend_from_slice(&0u32.to_le_bytes()); // nonce
        h
    }

    #[test]
    fn merkle_root_single_node() {
        let leaf = [0x42u8; 32];
        assert_eq!(compute_btc_merkle_root(leaf, &[], 0), leaf);
    }

    #[test]
    fn merkle_root_two_nodes_position_left() {
        let leaf = [0x11u8; 32];
        let sibling = [0x22u8; 32];
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&leaf);
        buf[32..].copy_from_slice(&sibling);
        let expected = dsha(&buf);
        assert_eq!(compute_btc_merkle_root(leaf, &[sibling], 0), expected);
    }

    #[test]
    fn merkle_root_two_nodes_position_right() {
        let leaf = [0x11u8; 32];
        let sibling = [0x22u8; 32];
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&sibling);
        buf[32..].copy_from_slice(&leaf);
        let expected = dsha(&buf);
        assert_eq!(compute_btc_merkle_root(leaf, &[sibling], 1), expected);
    }

    fn chain_iter(chain: &[Vec<u8>]) -> impl Iterator<Item = &[u8]> {
        chain.iter().map(Vec::as_slice)
    }

    #[test]
    fn header_chain_empty_chain_must_match_tip() {
        let h = make_header([0u8; 32], [0u8; 32]);
        let tip = block_hash(&h);
        let chain: [Vec<u8>; 0] = [];
        assert!(verify_header_chain(&h, chain_iter(&chain), tip).is_ok());
    }

    #[test]
    fn header_chain_two_link_chain() {
        let h0 = make_header([0u8; 32], [1u8; 32]);
        let h0_hash = block_hash(&h0);
        let h1 = make_header(h0_hash, [2u8; 32]);
        let h1_hash = block_hash(&h1);
        let h2 = make_header(h1_hash, [3u8; 32]);
        let tip = block_hash(&h2);

        let chain = vec![h1, h2];
        verify_header_chain(&h0, chain_iter(&chain), tip).expect("valid chain must verify");
    }

    #[test]
    fn header_chain_break_detected() {
        let h0 = make_header([0u8; 32], [1u8; 32]);
        let h1 = make_header([0xffu8; 32], [2u8; 32]); // wrong prev
        let tip_lie = block_hash(&h1);
        let chain = vec![h1];
        let err = verify_header_chain(&h0, chain_iter(&chain), tip_lie).unwrap_err();
        assert!(matches!(err, DaInclusionError::HeaderChainBreak { idx: 0 }));
    }

    #[test]
    fn header_chain_tip_mismatch_detected() {
        let h0 = make_header([0u8; 32], [1u8; 32]);
        let h0_hash = block_hash(&h0);
        let h1 = make_header(h0_hash, [2u8; 32]);
        let chain = vec![h1];
        let err = verify_header_chain(&h0, chain_iter(&chain), [0xaau8; 32]).unwrap_err();
        assert!(matches!(err, DaInclusionError::TipMismatch));
    }

    #[test]
    fn coinbase_inclusion_round_trip() {
        // Two-tx block where coinbase is at index 0.
        let coinbase_txid = [0xaau8; 32];
        let other_txid = [0xbbu8; 32];
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&coinbase_txid);
        buf[32..].copy_from_slice(&other_txid);
        let merkle_root = dsha(&buf);

        verify_coinbase_inclusion(coinbase_txid, &[other_txid], 0, merkle_root).unwrap();
    }

    #[test]
    fn coinbase_inclusion_rejects_wrong_root() {
        let err = verify_coinbase_inclusion([1u8; 32], &[[2u8; 32]], 0, [0u8; 32]).unwrap_err();
        assert!(matches!(err, DaInclusionError::CoinbaseMerkleMismatch));
    }

    #[test]
    fn wtxid_inclusion_round_trip() {
        // Witness tree with leaves [coinbase=0; wtxid; sibling].
        // Pad to power of 2 ⇒ 4 leaves: [0..0, wtxid, sibling, sibling]
        let coinbase_wtxid = [0u8; 32]; // BIP-141 zero leaf for coinbase
        let wtxid = [0xccu8; 32];
        let sibling_a = [0xddu8; 32];
        let sibling_b = [0xeeu8; 32];

        // Position 1 leaf path: pair with sibling at level 0 then with
        // hash(sibling_a, sibling_b) at level 1.
        let mut lvl0_left_buf = [0u8; 64];
        lvl0_left_buf[..32].copy_from_slice(&coinbase_wtxid);
        lvl0_left_buf[32..].copy_from_slice(&wtxid);
        let lvl0_left = dsha(&lvl0_left_buf);

        let mut lvl0_right_buf = [0u8; 64];
        lvl0_right_buf[..32].copy_from_slice(&sibling_a);
        lvl0_right_buf[32..].copy_from_slice(&sibling_b);
        let lvl0_right = dsha(&lvl0_right_buf);

        let mut root_buf = [0u8; 64];
        root_buf[..32].copy_from_slice(&lvl0_left);
        root_buf[32..].copy_from_slice(&lvl0_right);
        let witness_root = dsha(&root_buf);

        // wtxid sits at position 1; siblings = [coinbase_wtxid (left at lvl0), lvl0_right (right at
        // lvl1)].
        let siblings = vec![coinbase_wtxid, lvl0_right];
        let position = 1u32;

        let reserved = [0u8; 32];
        let coinbase = make_coinbase(witness_root, reserved);
        let coinbase_raw = serialize(&coinbase);

        verify_wtxid_inclusion(wtxid, &siblings, position, &coinbase_raw)
            .expect("valid wtxid proof must verify");
    }

    #[test]
    fn wtxid_inclusion_rejects_bad_proof() {
        let coinbase_wtxid = [0u8; 32];
        let wtxid = [0xccu8; 32];
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&coinbase_wtxid);
        buf[32..].copy_from_slice(&wtxid);
        let witness_root = dsha(&buf);

        let reserved = [0u8; 32];
        let coinbase = make_coinbase(witness_root, reserved);
        let coinbase_raw = serialize(&coinbase);

        // Tamper sibling.
        let bad_siblings = vec![[0xffu8; 32]];
        let err = verify_wtxid_inclusion(wtxid, &bad_siblings, 1, &coinbase_raw).unwrap_err();
        assert!(matches!(err, DaInclusionError::WitnessCommitmentMismatch));
    }

    #[test]
    fn wtxid_inclusion_rejects_coinbase_without_commitment() {
        // Coinbase without OP_RETURN witness commitment.
        let cb = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: {
                    let mut w = Witness::new();
                    w.push([0u8; 32]);
                    w
                },
            }],
            output: vec![],
        };
        let raw = serialize(&cb);
        let err = verify_wtxid_inclusion([0u8; 32], &[], 0, &raw).unwrap_err();
        assert!(matches!(err, DaInclusionError::MissingWitnessCommitment));
    }
}
