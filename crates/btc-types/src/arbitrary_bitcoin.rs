//! Arbitrary generation helpers for bitcoin types.
//!
//! Provides functions for generating arbitrary bitcoin types, for use in
//! manual [`Arbitrary`] implementations on structs that contain bitcoin fields.

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{Amount, OutPoint, ScriptBuf, TxOut, Txid, hashes::Hash};

/// Generates an arbitrary [`Txid`].
pub fn arbitrary_txid(u: &mut Unstructured<'_>) -> arbitrary::Result<Txid> {
    let bytes: [u8; 32] = u.arbitrary()?;
    Ok(Txid::from_byte_array(bytes))
}

/// Generates an arbitrary [`OutPoint`].
pub fn arbitrary_outpoint(u: &mut Unstructured<'_>) -> arbitrary::Result<OutPoint> {
    Ok(OutPoint {
        txid: arbitrary_txid(u)?,
        vout: u.arbitrary()?,
    })
}

/// Generates an arbitrary [`TxOut`] with a bounded script length.
pub fn arbitrary_txout(u: &mut Unstructured<'_>) -> arbitrary::Result<TxOut> {
    let value = Amount::from_sat(u.arbitrary()?);
    let script_len = usize::arbitrary(u)? % 100;
    let script_bytes = u.bytes(script_len)?;
    Ok(TxOut {
        value,
        script_pubkey: ScriptBuf::from(script_bytes.to_vec()),
    })
}

/// Generates an arbitrary [`ScriptBuf`] with a bounded length.
pub fn arbitrary_script_buf(u: &mut Unstructured<'_>) -> arbitrary::Result<ScriptBuf> {
    let script_len = usize::arbitrary(u)? % 100;
    let script_bytes = u.bytes(script_len)?;
    Ok(ScriptBuf::from(script_bytes.to_vec()))
}
