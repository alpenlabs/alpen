use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{
    absolute::LockTime,
    consensus::{deserialize, encode, serialize},
    hashes::Hash,
    transaction::Version,
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

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

impl<'a> Arbitrary<'a> for RawBitcoinTx {
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
            let txid = Txid::from_raw_hash(bitcoin::hashes::Hash::from_byte_array(txid_bytes));

            // Random vout
            let vout = u32::arbitrary(u)?;

            // Random scriptSig (bounded size)
            let script_sig_size = u.int_in_range::<usize>(0..=50)?;
            let script_sig_bytes = u.bytes(script_sig_size)?;
            let script_sig = ScriptBuf::from_bytes(script_sig_bytes.to_vec());

            inputs.push(TxIn {
                previous_output: OutPoint { txid, vout },
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
