use std::{
    fmt::Display,
    io::{self, Read, Write},
    iter::Sum,
    ops::Add,
};

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{
    absolute::LockTime,
    address::NetworkUnchecked,
    consensus::{deserialize, encode, serialize},
    hashes::{sha256d, Hash},
    key::{rand, Keypair, Parity, TapTweak},
    secp256k1::{SecretKey, XOnlyPublicKey, SECP256K1},
    taproot::{ControlBlock, LeafVersion, TaprootMerkleBranch},
    transaction::Version,
    Address, AddressType, Amount, Network, OutPoint, Psbt, ScriptBuf, Sequence, TapNodeHash,
    Transaction, TxIn, TxOut, Txid, Witness,
};
use bitcoin_bosd::Descriptor;
use borsh::{BorshDeserialize, BorshSerialize};
use rand::rngs::OsRng;
use serde::{de, Deserialize, Deserializer, Serialize};

use crate::{buf::Buf32, constants::HASH_SIZE, errors::ParseError};

/// L1 output reference.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct OutputRef(pub OutPoint);

impl From<OutPoint> for OutputRef {
    fn from(value: OutPoint) -> Self {
        Self(value)
    }
}

impl OutputRef {
    pub fn new(txid: Txid, vout: u32) -> Self {
        Self(OutPoint::new(txid, vout))
    }

    pub fn outpoint(&self) -> &OutPoint {
        &self.0
    }
}

// Implement BorshSerialize for the OutputRef wrapper.
impl BorshSerialize for OutputRef {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        // Serialize the transaction ID as bytes
        writer.write_all(&self.0.txid[..])?;

        // Serialize the output index as a little-endian 4-byte integer
        writer.write_all(&self.0.vout.to_le_bytes())?;
        Ok(())
    }
}

// Implement BorshDeserialize for the OutputRef wrapper.
impl BorshDeserialize for OutputRef {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, io::Error> {
        // Read 32 bytes for the transaction ID
        let mut txid_bytes = [0u8; HASH_SIZE];
        reader.read_exact(&mut txid_bytes)?;
        let txid = bitcoin::Txid::from_slice(&txid_bytes).expect("should be a valid txid (hash)");

        // Read 4 bytes for the output index
        let mut vout_bytes = [0u8; 4];
        reader.read_exact(&mut vout_bytes)?;
        let vout = u32::from_le_bytes(vout_bytes);

        Ok(OutputRef(OutPoint { txid, vout }))
    }
}

// Implement Arbitrary for the wrapper
impl<'a> Arbitrary<'a> for OutputRef {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate a random 32-byte array for the transaction ID (txid)
        let mut txid_bytes = [0u8; HASH_SIZE];
        u.fill_buffer(&mut txid_bytes)?;
        let txid_bytes = &txid_bytes[..];
        let hash = sha256d::Hash::from_slice(txid_bytes).unwrap();
        let txid = bitcoin::Txid::from_slice(&hash[..]).unwrap();

        // Generate a random 4-byte integer for the output index (vout)
        let vout = u.int_in_range(0..=u32::MAX)?;

        Ok(OutputRef(OutPoint { txid, vout }))
    }
}
/// A wrapper around the [`bitcoin::Address<NetworkChecked>`] type.
///
/// It's created in order to implement some useful traits on it such as
/// [`serde::Deserialize`], [`borsh::BorshSerialize`] and [`borsh::BorshDeserialize`].
// TODO: implement [`arbitrary::Arbitrary`]?
#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct BitcoinAddress {
    /// The [`bitcoin::Network`] that this address is valid in.
    network: Network,

    /// The actual [`Address`] that this type wraps.
    address: Address,
}

impl BitcoinAddress {
    /// Parses a [`BitcoinAddress`] from a string.
    pub fn parse(address_str: &str, network: Network) -> Result<Self, ParseError> {
        let address = address_str
            .parse::<Address<NetworkUnchecked>>()
            .map_err(ParseError::InvalidAddress)?;

        let checked_address = address
            .require_network(network)
            .map_err(ParseError::InvalidAddress)?;

        Ok(Self {
            network,
            address: checked_address,
        })
    }

    /// Parses a [`BitcoinAddress`] from raw bytes representation of a bitcoin Script.
    pub fn from_bytes(bytes: &[u8], network: Network) -> Result<Self, ParseError> {
        let script_buf = ScriptBuf::from_bytes(bytes.to_vec());
        let address = Address::from_script(&script_buf, network)?;
        Ok(Self { network, address })
    }

    pub fn from_descriptor(descriptor: &Descriptor, network: Network) -> Result<Self, ParseError> {
        let address = descriptor
            .to_address(network)
            .map_err(|err| ParseError::Descriptor(err.to_string()))?;
        Ok(Self { network, address })
    }
}

impl BitcoinAddress {
    pub fn address(&self) -> &Address {
        &self.address
    }

    pub fn network(&self) -> &Network {
        &self.network
    }
}

impl<'de> Deserialize<'de> for BitcoinAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct BitcoinAddressShim {
            network: Network,
            address: String,
        }

        let shim = BitcoinAddressShim::deserialize(deserializer)?;
        let address = shim
            .address
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| de::Error::custom("invalid bitcoin address"))?
            .require_network(shim.network)
            .map_err(|_| de::Error::custom("address invalid for given network"))?;

        Ok(BitcoinAddress {
            network: shim.network,
            address,
        })
    }
}

impl BorshSerialize for BitcoinAddress {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        let address_string = self.address.to_string();

        BorshSerialize::serialize(address_string.as_str(), writer)?;

        let network_byte = match self.network {
            Network::Bitcoin => 0u8,
            Network::Testnet => 1u8,
            Network::Signet => 2u8,
            Network::Regtest => 3u8,
            other => unreachable!("should handle new variant: {}", other),
        };

        BorshSerialize::serialize(&network_byte, writer)?;

        Ok(())
    }
}

impl BorshDeserialize for BitcoinAddress {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, io::Error> {
        let address_str = String::deserialize_reader(reader)?;

        let network_byte = u8::deserialize_reader(reader)?;
        let network = match network_byte {
            0u8 => Network::Bitcoin,
            1u8 => Network::Testnet,
            2u8 => Network::Signet,
            3u8 => Network::Regtest,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid network byte: {network_byte}"),
                ));
            }
        };

        let address = address_str
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid bitcoin address"))?
            .require_network(network)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "address invalid for given network",
                )
            })?;

        Ok(BitcoinAddress { address, network })
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
    PartialEq,
    Serialize,
)]
pub struct BitcoinAmount(u64);

impl Display for BitcoinAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
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
}

impl Add for BitcoinAmount {
    type Output = BitcoinAmount;

    fn add(self, rhs: Self) -> Self::Output {
        Self::from_sat(self.to_sat() + rhs.to_sat())
    }
}

impl Sum for BitcoinAmount {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self::from_sat(iter.map(|amt| amt.to_sat()).sum())
    }
}

/// [Borsh](borsh)-friendly Bitcoin [`Psbt`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BitcoinPsbt(Psbt);

impl BitcoinPsbt {
    pub fn inner(&self) -> &Psbt {
        &self.0
    }

    pub fn compute_txid(&self) -> Txid {
        self.0.unsigned_tx.compute_txid()
    }
}

impl From<Psbt> for BitcoinPsbt {
    fn from(value: Psbt) -> Self {
        Self(value)
    }
}

impl From<BitcoinPsbt> for Psbt {
    fn from(value: BitcoinPsbt) -> Self {
        value.0
    }
}

impl BorshSerialize for BitcoinPsbt {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize the PSBT using bitcoin's built-in serialization
        let psbt_bytes = self.0.serialize();
        // First, write the length of the serialized PSBT (as u32)
        BorshSerialize::serialize(&(psbt_bytes.len() as u32), writer)?;
        // Then, write the actual serialized PSBT bytes
        writer.write_all(&psbt_bytes)?;
        Ok(())
    }
}

impl BorshDeserialize for BitcoinPsbt {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // First, read the length of the PSBT (as u32)
        let len = u32::deserialize_reader(reader)? as usize;
        // Then, create a buffer to hold the PSBT bytes and read them
        let mut psbt_bytes = vec![0u8; len];
        reader.read_exact(&mut psbt_bytes)?;
        // Use the bitcoin crate's deserialize method to create a Psbt from the bytes
        let psbt = Psbt::deserialize(&psbt_bytes).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid PSBT data")
        })?;
        Ok(BitcoinPsbt(psbt))
    }
}

impl<'a> Arbitrary<'a> for BitcoinPsbt {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let num_outputs = u.arbitrary_len::<[u8; 32]>()? % 5;
        let mut output: Vec<TxOut> = vec![];

        for _ in 0..num_outputs {
            let txout = BitcoinTxOut::arbitrary(u)?;
            let txout = TxOut::from(txout);

            output.push(txout);
        }

        let tx = Transaction {
            version: Version(1),
            lock_time: LockTime::from_consensus(0),
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                witness: Witness::new(),
                sequence: Sequence(0),
                script_sig: ScriptBuf::new(),
            }],
            output,
        };

        let psbt = Psbt::from_unsigned_tx(tx).map_err(|_e| arbitrary::Error::IncorrectFormat)?;
        let psbt = BitcoinPsbt::from(psbt);

        Ok(psbt)
    }
}

/// [Borsh](borsh)-friendly Bitcoin [`Txid`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitcoinTxid(Txid);

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
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
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
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // First, read the length tag
        let len = u32::deserialize_reader(reader)? as usize;

        if len != HASH_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
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
        let txid = Txid::from(value);

        Ok(Self(txid))
    }
}

/// A wrapper around [`bitcoin::TxOut`] that implements some additional traits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BitcoinTxOut(TxOut);

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
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
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
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
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

/// The components required in the witness stack to spend a taproot output.
///
/// If a script-path path is being used, the witness stack needs the script being spent and the
/// control block in addition to the signature.
/// See [BIP 341](https://github.com/bitcoin/bips/blob/master/bip-0341.mediawiki#constructing-and-spending-taproot-outputs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaprootSpendPath {
    /// Use the keypath spend.
    ///
    /// This only requires the signature for the tweaked internal key and nothing else.
    Key,

    /// Use the script path spend.
    ///
    /// This requires the script being spent from as well as the [`ControlBlock`] in addition to
    /// the elements that fulfill the spending condition in the script.
    Script {
        script_buf: ScriptBuf,
        control_block: ControlBlock,
    },
}

impl BorshSerialize for TaprootSpendPath {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            TaprootSpendPath::Key => {
                // Variant index for Keypath is 0
                BorshSerialize::serialize(&0u32, writer)?;
            }
            TaprootSpendPath::Script {
                script_buf,
                control_block,
            } => {
                // Variant index for ScriptPath is 1
                BorshSerialize::serialize(&1u32, writer)?;

                // Serialize the ScriptBuf
                let script_bytes = script_buf.to_bytes();
                BorshSerialize::serialize(&(script_bytes.len() as u64), writer)?;
                writer.write_all(&script_bytes)?;

                // Serialize the ControlBlock using bitcoin's serialize method
                let control_block_bytes = control_block.serialize();
                BorshSerialize::serialize(&(control_block_bytes.len() as u64), writer)?;
                writer.write_all(&control_block_bytes)?;
            }
        }
        Ok(())
    }
}

// Implement BorshDeserialize for TaprootSpendInfo
impl BorshDeserialize for TaprootSpendPath {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // Deserialize the variant index
        let variant: u32 = BorshDeserialize::deserialize_reader(reader)?;
        match variant {
            0 => Ok(TaprootSpendPath::Key),
            1 => {
                // Deserialize the ScriptBuf
                let script_len = u64::deserialize_reader(reader)? as usize;
                let mut script_bytes = vec![0u8; script_len];
                reader.read_exact(&mut script_bytes)?;
                let script_buf = ScriptBuf::from(script_bytes);

                // Deserialize the ControlBlock
                let control_block_len = u64::deserialize_reader(reader)? as usize;
                let mut control_block_bytes = vec![0u8; control_block_len];
                reader.read_exact(&mut control_block_bytes)?;
                let control_block: ControlBlock = ControlBlock::decode(&control_block_bytes[..])
                    .map_err(|_| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid ControlBlock")
                    })?;

                Ok(TaprootSpendPath::Script {
                    script_buf,
                    control_block,
                })
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unknown variant for TaprootSpendInfo",
            )),
        }
    }
}

// Implement Arbitrary for TaprootSpendInfo
impl<'a> Arbitrary<'a> for TaprootSpendPath {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Randomly decide which variant to generate
        let variant = u.int_in_range(0..=1)?;
        match variant {
            0 => Ok(TaprootSpendPath::Key),
            1 => {
                // Arbitrary ScriptBuf (the script part of SpendInfo)
                let script_len = usize::arbitrary(u)? % 100; // Limit the length of the script for practicality
                let script_bytes = u.bytes(script_len)?; // Generate random bytes for the script
                let script_buf = ScriptBuf::from(script_bytes.to_vec());

                // Now we will manually generate the fields of the ControlBlock struct

                // Leaf version
                let leaf_version = LeafVersion::TapScript;

                // Output key parity (Even or Odd)
                let output_key_parity = if bool::arbitrary(u)? {
                    Parity::Even
                } else {
                    Parity::Odd
                };

                // Generate a random secret key and derive the internal key
                let secret_key = SecretKey::new(&mut OsRng);
                let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
                let (internal_key, _) = XOnlyPublicKey::from_keypair(&keypair);

                // Arbitrary Taproot merkle branch (vector of 32-byte hashes)
                const BRANCH_LENGTH: usize = 10;
                let mut tapnode_hashes: Vec<TapNodeHash> = Vec::with_capacity(BRANCH_LENGTH);
                for _ in 0..BRANCH_LENGTH {
                    let hash = TapNodeHash::from_slice(&<[u8; 32]>::arbitrary(u)?)
                        .map_err(|_e| arbitrary::Error::IncorrectFormat)?;
                    tapnode_hashes.push(hash);
                }

                let tapnode_hashes: &[TapNodeHash; BRANCH_LENGTH] =
                    &tapnode_hashes[..BRANCH_LENGTH].try_into().unwrap();

                let merkle_branch = TaprootMerkleBranch::from(*tapnode_hashes);

                // Construct the ControlBlock manually
                let control_block = ControlBlock {
                    leaf_version,
                    output_key_parity,
                    internal_key,
                    merkle_branch,
                };

                // Construct the ScriptPath variant
                Ok(TaprootSpendPath::Script {
                    script_buf,
                    control_block,
                })
            }
            _ => unreachable!(),
        }
    }
}

/// Outpoint of a bitcoin tx
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, BorshSerialize, BorshDeserialize)]
pub struct Outpoint {
    pub txid: Buf32,
    pub vout: u32,
}

/// A wrapper around [`Buf32`] for XOnly Schnorr taproot pubkeys.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct XOnlyPk(Buf32);

impl XOnlyPk {
    /// Construct a new [`XOnlyPk`] directly from a [`Buf32`].
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

    /// Convert a [`BitcoinAddress`] into a [`XOnlyPk`].
    pub fn from_address(checked_addr: &BitcoinAddress) -> Result<Self, ParseError> {
        let checked_addr = checked_addr.address();

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

    /// Convert the [`XOnlyPk`] to a `rust-bitcoin`'s [`XOnlyPublicKey`].
    pub fn to_xonly_public_key(&self) -> XOnlyPublicKey {
        XOnlyPublicKey::from_slice(self.0.as_bytes()).expect("XOnlyPk is valid")
    }

    /// Convert the [`XOnlyPk`] to an [`Address`].
    pub fn to_p2tr_address(&self, network: Network) -> Result<Address, ParseError> {
        let buf: [u8; 32] = self.0 .0;
        let pubkey = XOnlyPublicKey::from_slice(&buf)?;

        Ok(Address::p2tr_tweaked(
            pubkey.dangerous_assume_tweaked(),
            network,
        ))
    }

    /// Converts [`XOnlyPk`] to [`Descriptor`].
    pub fn to_descriptor(&self) -> Result<Descriptor, ParseError> {
        Descriptor::new_p2tr(&self.to_xonly_public_key().serialize())
            .map_err(|_| ParseError::InvalidPoint(self.0))
    }

    /// Checks if the [`Buf32`] is a valid [`XOnlyPublicKey`].
    fn is_valid_xonly_public_key(buf: &Buf32) -> bool {
        XOnlyPublicKey::from_slice(buf.as_bytes()).is_ok()
    }
}

impl TryFrom<XOnlyPk> for Descriptor {
    type Error = ParseError;

    fn try_from(value: XOnlyPk) -> Result<Self, Self::Error> {
        let inner_xonly_pk = XOnlyPublicKey::try_from(value.0)?;
        Ok(inner_xonly_pk.into())
    }
}

/// Represents a raw, byte-encoded Bitcoin transaction with custom [`Arbitrary`] support.
/// Provides conversions (via [`TryFrom`]) to and from [`Transaction`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct RawBitcoinTx(Vec<u8>);

impl RawBitcoinTx {
    /// Creates a new `RawBitcoinTx` from a raw byte vector.
    pub fn from_raw_bytes(bytes: Vec<u8>) -> Self {
        RawBitcoinTx(bytes)
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
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let script_bytes = self.0.to_bytes();
        BorshSerialize::serialize(&(script_bytes.len() as u32), writer)?;
        writer.write_all(&script_bytes)?;
        Ok(())
    }
}

// Implement BorshDeserialize for BitcoinScriptBuf
impl BorshDeserialize for BitcoinScriptBuf {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
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
    use std::io::Cursor;

    use arbitrary::{Arbitrary, Unstructured};
    use bitcoin::{
        hashes::Hash,
        key::Keypair,
        opcodes::all::OP_CHECKSIG,
        script::Builder,
        secp256k1::{Parity, SecretKey, SECP256K1},
        taproot::{ControlBlock, LeafVersion, TaprootBuilder, TaprootMerkleBranch},
        Address, Amount, Network, ScriptBuf, TapNodeHash, Transaction, TxOut, XOnlyPublicKey,
    };
    use bitcoin_bosd::DescriptorType;
    use rand::{rngs::OsRng, Rng};
    use strata_test_utils::ArbitraryGenerator;

    use super::{
        BitcoinAddress, BitcoinAmount, BitcoinScriptBuf, BitcoinTxOut, BitcoinTxid,
        BorshDeserialize, BorshSerialize, RawBitcoinTx, XOnlyPk,
    };
    use crate::{
        buf::Buf32,
        errors::ParseError,
        l1::{BitcoinPsbt, TaprootSpendPath},
    };

    #[test]
    fn test_parse_bitcoin_address_network() {
        let possible_networks = [
            // mainnet
            Network::Bitcoin,
            // testnets
            Network::Testnet,
            Network::Signet,
            Network::Regtest,
        ];

        let num_possible_networks = possible_networks.len();

        let (secret_key, _) = SECP256K1.generate_keypair(&mut OsRng);
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
        let (internal_key, _) = XOnlyPublicKey::from_keypair(&keypair);

        for network in possible_networks.iter() {
            // NOTE: only checking for P2TR addresses for now as those are the ones we use. Other
            // typs of addresses can also be checked but that shouldn't be necessary.
            let address = Address::p2tr(SECP256K1, internal_key, None, *network);
            let address_str = address.to_string();

            BitcoinAddress::parse(&address_str, *network).expect("address should parse");

            let invalid_network = match network {
                Network::Bitcoin => {
                    // get one of the testnets
                    let index = OsRng.gen_range(1..num_possible_networks);

                    possible_networks[index]
                }
                Network::Testnet | Network::Signet | Network::Regtest => Network::Bitcoin,
                other => unreachable!("this variant needs to be handled: {}", other),
            };

            assert!(
                BitcoinAddress::parse(&address_str, invalid_network)
                    .is_err_and(|e| matches!(e, ParseError::InvalidAddress(_))),
                "should error with ParseError::InvalidAddress if parse is passed an invalid address/network pair: {address_str}, {invalid_network}"
            );
        }
    }

    #[test]
    fn json_serialization_of_bitcoin_address_works() {
        // this is a random address
        // TODO: implement `Arbitrary` on `BitcoinAddress` and remove this hardcoded value
        let mainnet_addr = "bc1qpaj2e2ccwqvyzvsfhcyktulrjkkd28fg75wjuc";
        let network = Network::Bitcoin;

        let bitcoin_addr = BitcoinAddress::parse(mainnet_addr, network)
            .expect("address should be valid for the network");

        let serialized_bitcoin_addr =
            serde_json::to_string(&bitcoin_addr).expect("serialization should work");
        let deserialized_bitcoind_addr: BitcoinAddress =
            serde_json::from_str(&serialized_bitcoin_addr).expect("deserialization should work");

        assert_eq!(
            bitcoin_addr, deserialized_bitcoind_addr,
            "original and serialized addresses must be the same"
        );
    }

    #[test]
    fn borsh_serialization_of_bitcoin_address_works() {
        let mainnet_addr = "bc1qpaj2e2ccwqvyzvsfhcyktulrjkkd28fg75wjuc";
        let network = Network::Bitcoin;
        let original_addr: BitcoinAddress =
            BitcoinAddress::parse(mainnet_addr, network).expect("should be a valid address");

        let mut serialized_addr: Vec<u8> = vec![];
        original_addr
            .serialize(&mut serialized_addr)
            .expect("borsh serialization of bitcoin address must work");

        let deserialized = BitcoinAddress::try_from_slice(&serialized_addr);
        assert!(
            deserialized.is_ok(),
            "deserialization of bitcoin address should work but got: {:?}",
            deserialized.unwrap_err()
        );

        assert_eq!(
            deserialized.unwrap(),
            original_addr,
            "original address and deserialized address must be the same",
        );
    }

    #[test]
    fn test_borsh_serialization_of_multiple_addresses() {
        // Sample Bitcoin addresses
        let addresses = [
            "1BoatSLRHtKNngkdXEeobR76b53LETtpyT",
            "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
            "bc1qpaj2e2ccwqvyzvsfhcyktulrjkkd28fg75wjuc",
        ];

        let network = Network::Bitcoin;

        // Convert strings to BitcoinAddress instances
        let bitcoin_addresses: Vec<BitcoinAddress> = addresses
            .iter()
            .map(|s| {
                BitcoinAddress::parse(s, network)
                    .unwrap_or_else(|_e| panic!("random address {s} should be valid on: {network}"))
            })
            .collect();

        // Serialize the vector of BitcoinAddress instances
        let mut serialized = Vec::new();
        bitcoin_addresses
            .serialize(&mut serialized)
            .expect("serialization should work");

        // Attempt to deserialize back into a vector of BitcoinAddress instances
        let deserialized: Vec<BitcoinAddress> =
            Vec::try_from_slice(&serialized).expect("Deserialization failed");

        // Check that the deserialized addresses match the original
        assert_eq!(bitcoin_addresses, deserialized);
    }

    #[test]
    fn test_borsh_serialization_of_address_in_struct() {
        #[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
        struct Test {
            address: BitcoinAddress,
            other: u32,
        }

        let sample_addr = "bc1qpaj2e2ccwqvyzvsfhcyktulrjkkd28fg75wjuc";
        let network = Network::Bitcoin;
        let original = Test {
            other: 1,
            address: BitcoinAddress::parse(sample_addr, network).expect("should be valid address"),
        };

        let mut serialized = vec![];
        original
            .serialize(&mut serialized)
            .expect("should be able to serialize");

        let deserialized: Test = Test::try_from_slice(&serialized).expect("should deserialize");

        assert_eq!(
            deserialized, original,
            "deserialized and original structs with address should be the same"
        );
    }

    #[test]
    fn bitcoin_addr_to_taproot_pubkey_conversion_works() {
        let network = Network::Bitcoin;
        let (address, _) = get_taproot_address(network);

        let taproot_pubkey = XOnlyPk::from_address(&address);

        assert!(
            taproot_pubkey.is_ok(),
            "conversion from address to taproot pubkey failed"
        );

        let taproot_pubkey = taproot_pubkey.unwrap();
        let bitcoin_address = taproot_pubkey.to_p2tr_address(network);

        assert!(
            bitcoin_address.is_ok(),
            "conversion from taproot pubkey to address failed"
        );

        let bitcoin_address = bitcoin_address.unwrap();
        let address_str = bitcoin_address.to_string();

        let new_taproot_pubkey = XOnlyPk::from_address(
            &BitcoinAddress::parse(&address_str, network).expect("should be a valid address"),
        );

        assert_eq!(
            bitcoin_address,
            *address.address(),
            "converted and original addresses must be the same"
        );

        assert_eq!(
            taproot_pubkey,
            new_taproot_pubkey.unwrap(),
            "converted and original taproot pubkeys must be the same"
        );
    }

    #[test]
    #[should_panic(expected = "number of sats greater than u64::MAX")]
    fn bitcoinamount_should_handle_sats_exceeding_u64_max() {
        let bitcoins: u64 = u64::MAX / BitcoinAmount::SATS_FACTOR + 1;

        BitcoinAmount::from_int_btc(bitcoins);
    }

    fn get_taproot_address(network: Network) -> (BitcoinAddress, Option<TapNodeHash>) {
        let internal_pubkey = get_random_pubkey_from_slice(&[0x12; 32]);

        let pk1 = get_random_pubkey_from_slice(&[0x02; 32]);

        let mut script1 = ScriptBuf::new();
        script1.push_slice(pk1.serialize());
        script1.push_opcode(OP_CHECKSIG);

        let pk2 = get_random_pubkey_from_slice(&[0x05; 32]);

        let mut script2 = ScriptBuf::new();
        script2.push_slice(pk2.serialize());
        script2.push_opcode(OP_CHECKSIG);

        let taproot_builder = TaprootBuilder::new()
            .add_leaf(1, script1)
            .unwrap()
            .add_leaf(1, script2)
            .unwrap();

        let tree_info = taproot_builder
            .finalize(SECP256K1, internal_pubkey)
            .unwrap();
        let merkle_root = tree_info.merkle_root();

        let taproot_address = Address::p2tr(SECP256K1, internal_pubkey, merkle_root, network);

        (
            BitcoinAddress::parse(&taproot_address.to_string(), network).unwrap(),
            merkle_root,
        )
    }

    #[test]
    fn test_bitcoinpsbt_serialize_deserialize() {
        // Create an arbitrary PSBT
        let random_data = &[0u8; 1024];
        let mut unstructured = Unstructured::new(&random_data[..]);
        let bitcoin_psbt: BitcoinPsbt = BitcoinPsbt::arbitrary(&mut unstructured).unwrap();

        // Serialize the struct
        let mut serialized = vec![];
        bitcoin_psbt
            .serialize(&mut serialized)
            .expect("Serialization failed");

        // Deserialize the struct
        let deserialized: BitcoinPsbt =
            BitcoinPsbt::deserialize(&mut &serialized[..]).expect("Deserialization failed");

        // Ensure the deserialized PSBT matches the original
        assert_eq!(bitcoin_psbt.0, deserialized.0);
    }

    #[test]
    fn test_borsh_serialize_deserialize_keypath() {
        let original = TaprootSpendPath::Key;

        let mut serialized = vec![];
        BorshSerialize::serialize(&original, &mut serialized).expect("borsh serialization");

        let mut cursor = Cursor::new(serialized);
        let deserialized =
            TaprootSpendPath::deserialize_reader(&mut cursor).expect("borsh deserialization");

        match deserialized {
            TaprootSpendPath::Key => (),
            _ => panic!("Deserialized variant does not match original"),
        }
    }

    #[test]
    fn test_borsh_serialize_deserialize_scriptpath() {
        // Create a sample ScriptBuf
        let script_bytes = vec![0x51, 0x21, 0xFF]; // Example script
        let script_buf = ScriptBuf::from(script_bytes.clone());

        // Create a sample ControlBlock
        let leaf_version = LeafVersion::TapScript;
        let output_key_parity = Parity::Even;

        // Generate a random internal key
        let secret_key = SecretKey::new(&mut OsRng);
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
        let (internal_key, _) = XOnlyPublicKey::from_keypair(&keypair);

        // Create dummy TapNodeHash entries
        let tapnode_hashes = [TapNodeHash::from_byte_array([0u8; 32]); 10];

        let merkle_branch = TaprootMerkleBranch::from(tapnode_hashes);

        let control_block = ControlBlock {
            leaf_version,
            output_key_parity,
            internal_key,
            merkle_branch,
        };

        let original = TaprootSpendPath::Script {
            script_buf: script_buf.clone(),
            control_block: control_block.clone(),
        };

        let mut serialized = vec![];
        BorshSerialize::serialize(&original, &mut serialized).expect("borsh serialization");

        let mut cursor = Cursor::new(serialized);
        let deserialized =
            TaprootSpendPath::deserialize_reader(&mut cursor).expect("borsh deserialization");

        match deserialized {
            TaprootSpendPath::Script {
                script_buf: deserialized_script_buf,
                control_block: deserialized_control_block,
            } => {
                assert_eq!(script_buf, deserialized_script_buf, "ScriptBuf mismatch");

                // Compare ControlBlock fields
                assert_eq!(
                    control_block.leaf_version, deserialized_control_block.leaf_version,
                    "LeafVersion mismatch"
                );
                assert_eq!(
                    control_block.output_key_parity, deserialized_control_block.output_key_parity,
                    "OutputKeyParity mismatch"
                );
                assert_eq!(
                    control_block.internal_key, deserialized_control_block.internal_key,
                    "InternalKey mismatch"
                );
                assert_eq!(
                    control_block.merkle_branch, deserialized_control_block.merkle_branch,
                    "MerkleBranch mismatch"
                );
            }
            _ => panic!("Deserialized variant does not match original"),
        }
    }

    #[test]
    fn test_arbitrary_borsh_roundtrip() {
        // Generate arbitrary TaprootSpendInfo
        let data = vec![0u8; 1024];
        let mut u = Unstructured::new(&data);

        let original = TaprootSpendPath::arbitrary(&mut u).expect("Arbitrary generation failed");

        // Serialize
        let mut serialized = vec![];
        BorshSerialize::serialize(&original, &mut serialized).expect("borsh serialization");

        // Deserialize
        let mut cursor = Cursor::new(&serialized);
        let deserialized =
            TaprootSpendPath::deserialize_reader(&mut cursor).expect("borsh deserialization");

        // Assert equality by serializing both and comparing bytes
        let mut original_serialized = vec![];
        BorshSerialize::serialize(&original, &mut original_serialized)
            .expect("borsh serialization");

        let mut deserialized_serialized = vec![];
        BorshSerialize::serialize(&deserialized, &mut deserialized_serialized)
            .expect("borsh serialization of deserialized");

        assert_eq!(
            original_serialized, deserialized_serialized,
            "Original and deserialized serialized data do not match"
        );
    }

    #[test]
    fn test_bitcointxout_serialize_deserialize() {
        // Create a dummy TxOut with a simple script
        let script = Builder::new()
            .push_opcode(bitcoin::blockdata::opcodes::all::OP_CHECKSIG)
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

    fn get_random_pubkey_from_slice(buf: &[u8]) -> XOnlyPublicKey {
        let sk = SecretKey::from_slice(buf).unwrap();
        let keypair = Keypair::from_secret_key(SECP256K1, &sk);
        let (pk, _) = XOnlyPublicKey::from_keypair(&keypair);

        pk
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
        let xonly_pk = XOnlyPk::new(Buf32::from([2u8; 32])).unwrap();
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
}
