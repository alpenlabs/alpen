use std::{
    fmt::{self, Debug, Display},
    io::{Read, Write},
    str,
};

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{
    hashes::Hash,
    key::{rand, Keypair, Parity},
    secp256k1::{SecretKey, XOnlyPublicKey, SECP256K1},
    taproot::{ControlBlock, LeafVersion, TaprootMerkleBranch},
    Amount, ScriptBuf, TapNodeHash, TxOut,
};
use borsh::{BorshDeserialize, BorshSerialize};
use hex::encode_to_slice;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

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
                    let hash = TapNodeHash::from_byte_array(<[u8; 32]>::arbitrary(u)?);
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
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, BorshSerialize, BorshDeserialize)]
pub struct Outpoint {
    pub txid: Buf32,
    pub vout: u32,
}

// Custom debug implementation to print txid in little endian
impl Debug for Outpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut txid_buf = [0u8; 64];
        {
            let mut bytes = self.txid.0;
            bytes.reverse();
            encode_to_slice(bytes, &mut txid_buf).expect("buf: enc hex");
        }

        f.debug_struct("Outpoint")
            .field("txid", &unsafe { std::str::from_utf8_unchecked(&txid_buf) })
            .field("vout", &self.vout)
            .finish()
    }
}

// Custom display implementation to print txid in little endian
impl Display for Outpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut txid_buf = [0u8; 64];
        {
            let mut bytes = self.txid.0;
            bytes.reverse();
            encode_to_slice(bytes, &mut txid_buf).expect("buf: enc hex");
        }

        write!(
            f,
            "Outpoint {{ txid: {}, vout: {} }}",
            // SAFETY: hex encoding always produces valid UTF-8
            unsafe { str::from_utf8_unchecked(&txid_buf) },
            self.vout
        )
    }
}
