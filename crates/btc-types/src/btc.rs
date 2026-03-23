use std::{
    fmt::{self, Debug, Display},
    io::{self, Read, Write},
    ops,
};

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{
    Address, AddressType, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
    absolute::LockTime,
    consensus::{deserialize, encode, serialize},
    hashes::{Hash, sha256d},
    key::TapTweak,
    secp256k1::XOnlyPublicKey,
    transaction::Version,
};
use bitcoin_bosd::Descriptor;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode as SszDecodeTrait, DecodeError, Encode as SszEncodeTrait};
use ssz_derive::{Decode, Encode};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::{Buf32, impl_ssz_transparent_wrapper};

use crate::ParseError;

const HASH_SIZE: usize = 32;
const BITCOIN_OUTPOINT_LEN: usize = 36;
const BITCOIN_TXID_LEN: usize = 32;

/// L1 output reference.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct BitcoinOutPoint(pub OutPoint);

impl SszEncodeTrait for BitcoinOutPoint {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        BITCOIN_OUTPOINT_LEN
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.0.txid.to_byte_array());
        buf.extend_from_slice(&self.0.vout.to_le_bytes());
    }

    fn ssz_bytes_len(&self) -> usize {
        <Self as SszEncodeTrait>::ssz_fixed_len()
    }
}

impl SszDecodeTrait for BitcoinOutPoint {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        BITCOIN_OUTPOINT_LEN
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != <Self as SszDecodeTrait>::ssz_fixed_len() {
            return Err(DecodeError::InvalidByteLength {
                len: bytes.len(),
                expected: <Self as SszDecodeTrait>::ssz_fixed_len(),
            });
        }

        let txid = Txid::from_slice(&bytes[..BITCOIN_TXID_LEN])
            .map_err(|err| DecodeError::BytesInvalid(err.to_string()))?;
        let vout = u32::from_le_bytes(
            bytes[BITCOIN_TXID_LEN..<Self as SszDecodeTrait>::ssz_fixed_len()]
                .try_into()
                .expect("slice length is checked above"),
        );
        Ok(Self(OutPoint { txid, vout }))
    }
}

impl From<OutPoint> for BitcoinOutPoint {
    fn from(value: OutPoint) -> Self {
        Self(value)
    }
}

impl BitcoinOutPoint {
    pub fn new(txid: Txid, vout: u32) -> Self {
        Self(OutPoint::new(txid, vout))
    }

    pub fn outpoint(&self) -> &OutPoint {
        &self.0
    }
}

// Implement BorshSerialize for the BitcoinOutPoint wrapper.
impl BorshSerialize for BitcoinOutPoint {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        // Serialize the transaction ID as bytes
        writer.write_all(&self.0.txid[..])?;

        // Serialize the output index as a little-endian 4-byte integer
        writer.write_all(&self.0.vout.to_le_bytes())?;
        Ok(())
    }
}

// Implement BorshDeserialize for the BitcoinOutPoint wrapper.
impl BorshDeserialize for BitcoinOutPoint {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, io::Error> {
        // Read 32 bytes for the transaction ID
        let mut txid_bytes = [0u8; HASH_SIZE];
        reader.read_exact(&mut txid_bytes)?;
        let txid = bitcoin::Txid::from_slice(&txid_bytes).expect("should be a valid txid (hash)");

        // Read 4 bytes for the output index
        let mut vout_bytes = [0u8; 4];
        reader.read_exact(&mut vout_bytes)?;
        let vout = u32::from_le_bytes(vout_bytes);

        Ok(BitcoinOutPoint(OutPoint { txid, vout }))
    }
}

// Implement Arbitrary for the wrapper
impl<'a> Arbitrary<'a> for BitcoinOutPoint {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate a random 32-byte array for the transaction ID (txid)
        let mut txid_bytes = [0u8; HASH_SIZE];
        u.fill_buffer(&mut txid_bytes)?;
        let txid_bytes = &txid_bytes[..];
        let hash = sha256d::Hash::from_slice(txid_bytes).unwrap();
        let txid = bitcoin::Txid::from_slice(&hash[..]).unwrap();

        // Generate a random 4-byte integer for the output index (vout)
        let vout = u.int_in_range(0..=u32::MAX)?;

        Ok(BitcoinOutPoint(OutPoint { txid, vout }))
    }
}
/// A wrapper for bitcoin amount in sats similar to the implementation in [`bitcoin::Amount`].
///
/// NOTE: This wrapper has been created so that we can implement `Borsh*` traits on it.
#[derive(
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Encode,
    Decode,
)]
pub struct BitcoinAmount(u64);

impl Display for BitcoinAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for BitcoinAmount {
    fn default() -> Self {
        Self::ZERO
    }
}

impl From<Amount> for BitcoinAmount {
    fn from(value: Amount) -> Self {
        Self::from_sat(value.to_sat())
    }
}

impl From<BitcoinAmount> for Amount {
    fn from(value: BitcoinAmount) -> Self {
        Self::from_sat(value.to_sat())
    }
}

impl From<u64> for BitcoinAmount {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<BitcoinAmount> for u64 {
    fn from(value: BitcoinAmount) -> Self {
        value.0
    }
}

impl ops::Deref for BitcoinAmount {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for BitcoinAmount {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Codec for BitcoinAmount {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Ok(Self(u64::decode(dec)?))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.0.encode(enc)
    }
}

impl_ssz_transparent_wrapper!(BitcoinAmount, u64);

impl BitcoinAmount {
    // The zero amount.
    pub const ZERO: BitcoinAmount = Self(0);
    /// The maximum value allowed as an amount. Useful for sanity checking.
    pub const MAX_MONEY: BitcoinAmount = Self::from_int_btc(21_000_000);
    /// The minimum value of an amount.
    pub const MIN: BitcoinAmount = Self::ZERO;
    /// The maximum value of an amount.
    pub const MAX: BitcoinAmount = Self(u64::MAX);
    /// The number of bytes that an amount contributes to the size of a transaction.
    /// Serialized length of a u64.
    pub const SIZE: usize = 8;

    /// The number of sats in 1 bitcoin.
    pub const SATS_FACTOR: u64 = 100_000_000;

    /// Get the number of sats in this [`BitcoinAmount`].
    pub fn to_sat(&self) -> u64 {
        self.0
    }

    /// Create a [`BitcoinAmount`] with sats precision and the given number of sats.
    pub const fn from_sat(value: u64) -> Self {
        Self(value)
    }

    /// Convert from a value strataing integer values of bitcoins to a [`BitcoinAmount`]
    /// in const context.
    ///
    /// ## Panics
    ///
    /// The function panics if the argument multiplied by the number of sats
    /// per bitcoin overflows a u64 type, or is greater than [`BitcoinAmount::MAX_MONEY`].
    pub const fn from_int_btc(btc: u64) -> Self {
        match btc.checked_mul(Self::SATS_FACTOR) {
            Some(amount) => Self::from_sat(amount),
            None => {
                panic!("number of sats greater than u64::MAX");
            }
        }
    }

    /// Checked addition. Returns [`None`] if overflow occurred.
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self::from_sat)
    }

    /// Checked subtraction. Returns [`None`] if overflow occurred.
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self::from_sat)
    }

    /// Checked multiplication. Returns [`None`] if overflow occurred.
    pub fn checked_mul(self, rhs: u64) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self::from_sat)
    }

    /// Checked division. Returns [`None`] if `rhs == 0`.
    pub fn checked_div(self, rhs: u64) -> Option<Self> {
        self.0.checked_div(rhs).map(Self::from_sat)
    }

    /// Saturating subtraction. Computes `self - rhs`, returning [`Self::ZERO`] if overflow
    /// occurred.
    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self::from_sat(self.to_sat().saturating_sub(rhs.to_sat()))
    }

    /// Saturating addition. Computes `self + rhs`, saturating at the numeric bounds.
    pub fn saturating_add(self, rhs: Self) -> Self {
        Self::from_sat(self.to_sat().saturating_add(rhs.to_sat()))
    }

    /// Returns a zero amount.
    pub const fn zero() -> Self {
        Self::ZERO
    }

    /// Returns true if the amount is zero.
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// Sums an iterator of [`BitcoinAmount`] values.
    pub fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |acc, amt| {
            acc.checked_add(amt)
                .expect("BitcoinAmount overflow during sum")
        })
    }
}

/// [Borsh](borsh)-friendly Bitcoin [`Txid`].
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitcoinTxid(Txid);

impl SszEncodeTrait for BitcoinTxid {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        BITCOIN_TXID_LEN
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.0.to_byte_array());
    }

    fn ssz_bytes_len(&self) -> usize {
        <Self as SszEncodeTrait>::ssz_fixed_len()
    }
}

impl SszDecodeTrait for BitcoinTxid {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        BITCOIN_TXID_LEN
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let serialized = <[u8; BITCOIN_TXID_LEN]>::from_ssz_bytes(bytes)?;
        Ok(Self(Txid::from_byte_array(serialized)))
    }
}

impl From<Txid> for BitcoinTxid {
    fn from(value: Txid) -> Self {
        Self(value)
    }
}

impl From<BitcoinTxid> for Txid {
    fn from(value: BitcoinTxid) -> Self {
        value.0
    }
}

impl BitcoinTxid {
    /// Creates a new [`BitcoinTxid`] from a [`Txid`].
    ///
    /// # Notes
    ///
    /// [`Txid`] is [`Copy`].
    pub fn new(txid: &Txid) -> Self {
        BitcoinTxid(*txid)
    }

    /// Gets the inner Bitcoin [`Txid`]
    pub fn inner(&self) -> Txid {
        self.0
    }

    /// Gets the inner Bitcoin [`Txid`] as raw bytes [`Buf32`].
    pub fn inner_raw(&self) -> Buf32 {
        self.0.as_raw_hash().to_byte_array().into()
    }
}

impl BorshSerialize for BitcoinTxid {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Serialize the txid using bitcoin's built-in serialization
        let txid_bytes = self.0.to_byte_array();
        // First, write the length of the serialized txid (as u32)
        BorshSerialize::serialize(&(32_u32), writer)?;
        // Then, write the actual serialized PSBT bytes
        writer.write_all(&txid_bytes)?;
        Ok(())
    }
}

impl BorshDeserialize for BitcoinTxid {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        // First, read the length tag
        let len = u32::deserialize_reader(reader)? as usize;

        if len != HASH_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid Txid size, expected: {HASH_SIZE}, got: {len}"),
            ));
        }

        // First, create a buffer to hold the txid bytes and read them
        let mut txid_bytes = [0u8; HASH_SIZE];
        reader.read_exact(&mut txid_bytes)?;
        // Use the bitcoin crate's deserialize method to create a Psbt from the bytes
        let txid = Txid::from_byte_array(txid_bytes);
        Ok(BitcoinTxid(txid))
    }
}

impl<'a> Arbitrary<'a> for BitcoinTxid {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let value = Buf32::arbitrary(u)?;
        let txid = Txid::from_byte_array(value.0);

        Ok(Self(txid))
    }
}

/// A wrapper around [`bitcoin::TxOut`] that implements some additional traits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitcoinTxOut(TxOut);

impl SszEncodeTrait for BitcoinTxOut {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        (self.0.value.to_sat(), self.0.script_pubkey.to_bytes()).ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        (self.0.value.to_sat(), self.0.script_pubkey.to_bytes()).ssz_bytes_len()
    }
}

impl SszDecodeTrait for BitcoinTxOut {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let (value, script_pubkey): (u64, Vec<u8>) = <(u64, Vec<u8>)>::from_ssz_bytes(bytes)?;
        Ok(Self(TxOut {
            value: Amount::from_sat(value),
            script_pubkey: ScriptBuf::from(script_pubkey),
        }))
    }
}

impl BitcoinTxOut {
    pub fn inner(&self) -> &TxOut {
        &self.0
    }
}

impl From<TxOut> for BitcoinTxOut {
    fn from(value: TxOut) -> Self {
        Self(value)
    }
}

impl From<BitcoinTxOut> for TxOut {
    fn from(value: BitcoinTxOut) -> Self {
        value.0
    }
}

// Implement BorshSerialize for BitcoinTxOut
impl BorshSerialize for BitcoinTxOut {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Serialize the value (u64)
        BorshSerialize::serialize(&self.0.value.to_sat(), writer)?;

        // Serialize the script_pubkey (ScriptBuf)
        let script_bytes = self.0.script_pubkey.to_bytes();
        BorshSerialize::serialize(&(script_bytes.len() as u64), writer)?;
        writer.write_all(&script_bytes)?;

        Ok(())
    }
}

// Implement BorshDeserialize for BitcoinTxOut
impl BorshDeserialize for BitcoinTxOut {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        // Deserialize the value (u64)
        let value = u64::deserialize_reader(reader)?;

        // Deserialize the script_pubkey (ScriptBuf)
        let script_len = u64::deserialize_reader(reader)? as usize;
        let mut script_bytes = vec![0u8; script_len];
        reader.read_exact(&mut script_bytes)?;
        let script_pubkey = ScriptBuf::from(script_bytes);

        Ok(BitcoinTxOut(TxOut {
            value: Amount::from_sat(value),
            script_pubkey,
        }))
    }
}

/// Implement Arbitrary for ArbitraryTxOut
impl<'a> Arbitrary<'a> for BitcoinTxOut {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate arbitrary value and script for the TxOut
        let value = u64::arbitrary(u)?;
        let script_len = usize::arbitrary(u)? % 100; // Limit script length
        let script_bytes = u.bytes(script_len)?;
        let script_pubkey = ScriptBuf::from(script_bytes.to_vec());

        Ok(Self(TxOut {
            value: Amount::from_sat(value),
            script_pubkey,
        }))
    }
}

/// A wrapper around [`Buf32`] for XOnly Schnorr taproot pubkeys.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
pub struct BitcoinXOnlyPublicKey(Buf32);

impl BitcoinXOnlyPublicKey {
    /// Construct a new [`BitcoinXOnlyPublicKey`] directly from a [`Buf32`].
    pub fn new(val: Buf32) -> Result<Self, ParseError> {
        if Self::is_valid_xonly_public_key(&val) {
            Ok(Self(val))
        } else {
            Err(ParseError::InvalidPoint(val))
        }
    }

    /// Get the underlying [`Buf32`].
    pub fn inner(&self) -> &Buf32 {
        &self.0
    }

    /// Convert a [`Address`] into a [`BitcoinXOnlyPublicKey`].
    pub fn from_address(checked_addr: &Address) -> Result<Self, ParseError> {
        if let Some(AddressType::P2tr) = checked_addr.address_type() {
            let script_pubkey = checked_addr.script_pubkey();

            // skip the version and length bytes
            let pubkey_bytes = &script_pubkey.as_bytes()[2..34];
            let output_key: XOnlyPublicKey = XOnlyPublicKey::from_slice(pubkey_bytes)?;

            Ok(Self(Buf32(output_key.serialize())))
        } else {
            Err(ParseError::UnsupportedAddress(checked_addr.address_type()))
        }
    }

    /// Convert the [`BitcoinXOnlyPublicKey`] to a `rust-bitcoin`'s [`XOnlyPublicKey`].
    pub fn to_xonly_public_key(&self) -> XOnlyPublicKey {
        XOnlyPublicKey::from_slice(self.0.as_bytes()).expect("BitcoinXOnlyPublicKey is valid")
    }

    /// Convert the [`BitcoinXOnlyPublicKey`] to an [`Address`].
    pub fn to_p2tr_address(&self, network: Network) -> Result<Address, ParseError> {
        let buf: [u8; 32] = self.0.0;
        let pubkey = XOnlyPublicKey::from_slice(&buf)?;

        Ok(Address::p2tr_tweaked(
            pubkey.dangerous_assume_tweaked(),
            network,
        ))
    }

    /// Converts [`BitcoinXOnlyPublicKey`] to [`Descriptor`].
    pub fn to_descriptor(&self) -> Result<Descriptor, ParseError> {
        Descriptor::new_p2tr(&self.to_xonly_public_key().serialize())
            .map_err(|_| ParseError::InvalidPoint(self.0))
    }

    /// Checks if the [`Buf32`] is a valid [`XOnlyPublicKey`].
    fn is_valid_xonly_public_key(buf: &Buf32) -> bool {
        XOnlyPublicKey::from_slice(buf.as_bytes()).is_ok()
    }
}

impl From<XOnlyPublicKey> for BitcoinXOnlyPublicKey {
    fn from(value: XOnlyPublicKey) -> Self {
        Self(Buf32(value.serialize()))
    }
}

impl TryFrom<BitcoinXOnlyPublicKey> for Descriptor {
    type Error = ParseError;

    fn try_from(value: BitcoinXOnlyPublicKey) -> Result<Self, Self::Error> {
        value.to_descriptor()
    }
}

impl_ssz_transparent_wrapper!(BitcoinXOnlyPublicKey, Buf32);

/// Represents a raw, byte-encoded Bitcoin transaction with custom [`Arbitrary`] support.
/// Provides conversions (via [`TryFrom`]) to and from [`Transaction`].
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
    Encode,
    Decode,
)]
pub struct RawBitcoinTx(Vec<u8>);

impl RawBitcoinTx {
    /// Creates a new `RawBitcoinTx` from a raw byte vector.
    pub fn from_raw_bytes(bytes: Vec<u8>) -> Self {
        RawBitcoinTx(bytes)
    }

    /// Returns the raw serialized transaction bytes.
    pub fn as_raw_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the wrapper and returns the raw serialized transaction bytes.
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl From<Transaction> for RawBitcoinTx {
    fn from(value: Transaction) -> Self {
        Self(serialize(&value))
    }
}

impl TryFrom<RawBitcoinTx> for Transaction {
    type Error = encode::Error;
    fn try_from(value: RawBitcoinTx) -> Result<Self, Self::Error> {
        deserialize(&value.0)
    }
}

impl TryFrom<&RawBitcoinTx> for Transaction {
    type Error = encode::Error;
    fn try_from(value: &RawBitcoinTx) -> Result<Self, Self::Error> {
        deserialize(&value.0)
    }
}

impl<'a> arbitrary::Arbitrary<'a> for RawBitcoinTx {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Random number of inputs and outputs (bounded for simplicity)
        let input_count = u.int_in_range::<usize>(0..=4)?;
        let output_count = u.int_in_range::<usize>(0..=4)?;

        // Build random inputs
        let mut inputs = Vec::with_capacity(input_count);
        for _ in 0..input_count {
            // Random 32-byte TXID
            let mut txid_bytes = [0u8; 32];
            u.fill_buffer(&mut txid_bytes)?;

            // Random vout
            let vout = u32::arbitrary(u)?;

            // Random scriptSig (bounded size)
            let script_sig_size = u.int_in_range::<usize>(0..=50)?;
            let script_sig_bytes = u.bytes(script_sig_size)?;
            let script_sig = ScriptBuf::from_bytes(script_sig_bytes.to_vec());

            inputs.push(TxIn {
                previous_output: OutPoint {
                    txid: Txid::from_byte_array(txid_bytes),
                    vout,
                },
                script_sig,
                sequence: Sequence::MAX,
                witness: Witness::default(), // or generate random witness if desired
            });
        }

        // Build random outputs
        let mut outputs = Vec::with_capacity(output_count);
        for _ in 0..output_count {
            // Random value (in satoshis)
            let value = Amount::from_sat(u64::arbitrary(u)?);

            // Random scriptPubKey (bounded size)
            let script_pubkey_size = u.int_in_range::<usize>(0..=50)?;
            let script_pubkey_bytes = u.bytes(script_pubkey_size)?;
            let script_pubkey = ScriptBuf::from(script_pubkey_bytes.to_vec());

            outputs.push(TxOut {
                value,
                script_pubkey,
            });
        }

        // Construct the transaction
        let tx = Transaction {
            version: Version::ONE,
            lock_time: LockTime::ZERO,
            input: inputs,
            output: outputs,
        };

        Ok(tx.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BitcoinScriptBuf(ScriptBuf);

impl SszEncodeTrait for BitcoinScriptBuf {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.0.to_bytes().ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        self.0.to_bytes().ssz_bytes_len()
    }
}

impl SszDecodeTrait for BitcoinScriptBuf {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        Vec::<u8>::from_ssz_bytes(bytes).map(|bytes| Self(ScriptBuf::from(bytes)))
    }
}

impl BitcoinScriptBuf {
    pub fn inner(&self) -> &ScriptBuf {
        &self.0
    }
}

impl From<ScriptBuf> for BitcoinScriptBuf {
    fn from(value: ScriptBuf) -> Self {
        Self(value)
    }
}

// Implement BorshSerialize for BitcoinScriptBuf
impl BorshSerialize for BitcoinScriptBuf {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let script_bytes = self.0.to_bytes();
        BorshSerialize::serialize(&(script_bytes.len() as u32), writer)?;
        writer.write_all(&script_bytes)?;
        Ok(())
    }
}

// Implement BorshDeserialize for BitcoinScriptBuf
impl BorshDeserialize for BitcoinScriptBuf {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let script_len = u32::deserialize_reader(reader)? as usize;
        let mut script_bytes = vec![0u8; script_len];
        reader.read_exact(&mut script_bytes)?;
        let script_pubkey = ScriptBuf::from(script_bytes);

        Ok(BitcoinScriptBuf(script_pubkey))
    }
}

impl<'a> Arbitrary<'a> for BitcoinScriptBuf {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate arbitrary script
        let script_len = usize::arbitrary(u)? % 100; // Limit script length
        let script_bytes = u.bytes(script_len)?;
        let script = ScriptBuf::from(script_bytes.to_vec());

        Ok(Self(script))
    }
}

#[cfg(test)]
mod tests {

    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Transaction, TxOut, Txid,
        hashes::Hash,
        opcodes::{self},
        script::Builder,
    };
    use bitcoin_bosd::DescriptorType;
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_identifiers::Buf32;
    use strata_test_utils::ArbitraryGenerator;
    use strata_test_utils_ssz::ssz_proptest;

    use super::{
        BitcoinAmount, BitcoinOutPoint, BitcoinScriptBuf, BitcoinTxOut, BitcoinTxid,
        BitcoinXOnlyPublicKey, BorshDeserialize, BorshSerialize, RawBitcoinTx,
    };

    #[test]
    #[should_panic(expected = "number of sats greater than u64::MAX")]
    fn bitcoinamount_should_handle_sats_exceeding_u64_max() {
        let bitcoins: u64 = u64::MAX / BitcoinAmount::SATS_FACTOR + 1;

        BitcoinAmount::from_int_btc(bitcoins);
    }

    #[test]
    fn test_bitcointxout_serialize_deserialize() {
        // Create a dummy TxOut with a simple script
        let script = Builder::new()
            .push_opcode(opcodes::all::OP_CHECKSIG)
            .into_script();
        let tx_out = TxOut {
            value: Amount::from_sat(1000),
            script_pubkey: script,
        };

        let bitcoin_tx_out = BitcoinTxOut(tx_out);

        // Serialize the BitcoinTxOut struct
        let mut serialized = vec![];
        bitcoin_tx_out
            .serialize(&mut serialized)
            .expect("Serialization failed");

        // Deserialize the BitcoinTxOut struct
        let deserialized: BitcoinTxOut =
            BitcoinTxOut::deserialize(&mut &serialized[..]).expect("Deserialization failed");

        // Ensure the deserialized BitcoinTxOut matches the original
        assert_eq!(bitcoin_tx_out.0.value, deserialized.0.value);
        assert_eq!(bitcoin_tx_out.0.script_pubkey, deserialized.0.script_pubkey);
    }

    #[test]
    fn test_bitcoin_txid_serialize_deserialize() {
        let mut generator = ArbitraryGenerator::new();
        let txid: BitcoinTxid = generator.generate();

        let serialized_txid =
            borsh::to_vec::<BitcoinTxid>(&txid).expect("should be able to serialize BitcoinTxid");
        let deserialized_txid = borsh::from_slice::<BitcoinTxid>(&serialized_txid)
            .expect("should be able to deserialize BitcoinTxid");

        assert_eq!(
            deserialized_txid, txid,
            "original and deserialized txid must be the same"
        );
    }

    proptest! {
        #[test]
        fn bitcoin_outpoint_ssz_roundtrip(txid_bytes in any::<[u8; 32]>(), vout in any::<u32>()) {
            let outpoint = BitcoinOutPoint(OutPoint {
                txid: Txid::from_byte_array(txid_bytes),
                vout,
            });

            let encoded = outpoint.as_ssz_bytes();
            let decoded = BitcoinOutPoint::from_ssz_bytes(&encoded).unwrap();

            prop_assert_eq!(decoded, outpoint);
        }

        #[test]
        fn bitcoin_txid_ssz_roundtrip(txid_bytes in any::<[u8; 32]>()) {
            let txid = BitcoinTxid::from(Txid::from_byte_array(txid_bytes));

            let encoded = txid.as_ssz_bytes();
            let decoded = BitcoinTxid::from_ssz_bytes(&encoded).unwrap();

            prop_assert_eq!(decoded, txid);
        }

        #[test]
        fn bitcoin_txout_ssz_roundtrip(
            value in any::<u64>(),
            script_pubkey in prop::collection::vec(any::<u8>(), 0..100),
        ) {
            let tx_out = BitcoinTxOut(TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::from_bytes(script_pubkey),
            });

            let encoded = tx_out.as_ssz_bytes();
            let decoded = BitcoinTxOut::from_ssz_bytes(&encoded).unwrap();

            prop_assert_eq!(decoded, tx_out);
        }
    }

    #[test]
    fn test_bitcoin_tx_arbitrary_generation() {
        let mut generator = ArbitraryGenerator::new();
        let raw_tx: RawBitcoinTx = generator.generate();
        let _: Transaction = raw_tx.try_into().expect("should generate valid tx");

        let raw_tx = RawBitcoinTx::from_raw_bytes(generator.generate());
        let res: Result<Transaction, _> = raw_tx.try_into();
        assert!(res.is_err());
    }

    #[test]
    fn test_xonly_pk_to_descriptor() {
        let xonly_pk = BitcoinXOnlyPublicKey::new(Buf32::from([2u8; 32])).unwrap();
        let descriptor = xonly_pk.to_descriptor().unwrap();
        assert_eq!(descriptor.type_tag(), DescriptorType::P2tr);

        let payload = descriptor.payload();
        assert_eq!(payload.len(), 32);
        assert_eq!(payload, xonly_pk.0.as_bytes());
    }

    #[test]
    fn test_bitcoin_scriptbuf_serialize_deserialize() {
        let mut generator = ArbitraryGenerator::new();
        let scriptbuf: BitcoinScriptBuf = generator.generate();

        let serialized_scriptbuf = borsh::to_vec(&scriptbuf).unwrap();
        let deserialized_scriptbuf: BitcoinScriptBuf =
            borsh::from_slice(&serialized_scriptbuf).unwrap();

        assert_eq!(
            scriptbuf.0, deserialized_scriptbuf.0,
            "original and deserialized scriptbuf must be the same"
        );

        // Test with an empty script
        let scriptbuf: BitcoinScriptBuf = BitcoinScriptBuf(ScriptBuf::new());
        let serialized_scriptbuf = borsh::to_vec(&scriptbuf).unwrap();
        let deserialized_scriptbuf: BitcoinScriptBuf =
            borsh::from_slice(&serialized_scriptbuf).unwrap();

        assert_eq!(
            scriptbuf.0, deserialized_scriptbuf.0,
            "original and deserialized scriptbuf must be the same"
        );

        // Test with a more complex script.
        let script: ScriptBuf = ScriptBuf::from_bytes(vec![0x51, 0x21, 0xFF]); // Example script

        let scriptbuf: BitcoinScriptBuf = BitcoinScriptBuf(script);

        let serialized_scriptbuf = borsh::to_vec(&scriptbuf).unwrap();
        let deserialized_scriptbuf: BitcoinScriptBuf =
            borsh::from_slice(&serialized_scriptbuf).unwrap();

        assert_eq!(
            scriptbuf.0, deserialized_scriptbuf.0,
            "original and deserialized scriptbuf must be the same"
        );
    }

    // Property-based tests for BitcoinAmount SSZ serialization
    ssz_proptest!(
        BitcoinAmount,
        any::<u64>(),
        transparent_wrapper_of(u64, from_sat)
    );

    #[test]
    fn test_bitcoin_amount_zero_ssz() {
        let zero = BitcoinAmount::zero();
        let encoded = zero.as_ssz_bytes();
        let decoded = BitcoinAmount::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(zero, decoded);
        assert!(decoded.is_zero());
    }
}
