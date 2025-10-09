use std::io::{Read, Write};

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{
    absolute::LockTime, transaction::Version, OutPoint, Psbt, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Txid, Witness,
};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::BitcoinTxOut;

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
