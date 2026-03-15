//! Borsh serialization helpers for bitcoin types using consensus encoding.
//!
//! Provides serialization/deserialization functions for use with
//! `#[borsh(serialize_with, deserialize_with)]` field attributes,
//! allowing bitcoin types to be used directly in Borsh-derived structs.

use std::io::{self, Read, Write};

use borsh::{BorshDeserialize, BorshSerialize};

/// Generates a `Vec<T>` borsh serializer module that delegates to an element-level serializer.
macro_rules! borsh_vec_helper {
    ($mod_name:ident, $type:ty, $elem_mod:ident) => {
        pub mod $mod_name {
            use super::*;

            pub fn serialize<W: Write>(val: &Vec<$type>, writer: &mut W) -> io::Result<()> {
                BorshSerialize::serialize(&(val.len() as u32), writer)?;
                for v in val {
                    $elem_mod::serialize(v, writer)?;
                }
                Ok(())
            }

            pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<Vec<$type>> {
                let len = u32::deserialize_reader(reader)? as usize;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push($elem_mod::deserialize(reader)?);
                }
                Ok(items)
            }
        }
    };
}

/// Generates an `Option<T>` borsh serializer module that delegates to an element-level serializer.
macro_rules! borsh_option_helper {
    ($mod_name:ident, $type:ty, $elem_mod:ident) => {
        pub mod $mod_name {
            use super::*;

            pub fn serialize<W: Write>(val: &Option<$type>, writer: &mut W) -> io::Result<()> {
                match val {
                    Some(v) => {
                        BorshSerialize::serialize(&1u8, writer)?;
                        $elem_mod::serialize(v, writer)
                    }
                    None => BorshSerialize::serialize(&0u8, writer),
                }
            }

            pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<Option<$type>> {
                let tag = u8::deserialize_reader(reader)?;
                match tag {
                    0 => Ok(None),
                    1 => Ok(Some($elem_mod::deserialize(reader)?)),
                    _ => Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid Option tag",
                    )),
                }
            }
        }
    };
}

/// Borsh helpers for [`bitcoin::Txid`] fields.
pub mod txid {
    use bitcoin::{Txid, hashes::Hash};

    use super::*;

    pub fn serialize<W: Write>(val: &Txid, writer: &mut W) -> io::Result<()> {
        writer.write_all(&val.to_byte_array())
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<Txid> {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf)?;
        Ok(Txid::from_byte_array(buf))
    }
}

borsh_vec_helper!(vec_txid, bitcoin::Txid, txid);

/// Borsh helpers for [`bitcoin::OutPoint`] fields using consensus encoding.
pub mod outpoint {
    use bitcoin::{OutPoint, consensus};

    use super::*;

    /// OutPoint consensus encoding is 32 bytes txid + 4 bytes vout = 36 bytes.
    const OUTPOINT_SIZE: usize = 36;

    pub fn serialize<W: Write>(val: &OutPoint, writer: &mut W) -> io::Result<()> {
        let bytes = consensus::serialize(val);
        writer.write_all(&bytes)
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<OutPoint> {
        let mut buf = [0u8; OUTPOINT_SIZE];
        reader.read_exact(&mut buf)?;
        consensus::deserialize(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// Borsh helpers for [`bitcoin::TxOut`] fields using consensus encoding.
pub mod txout {
    use bitcoin::{TxOut, consensus};

    use super::*;

    pub fn serialize<W: Write>(val: &TxOut, writer: &mut W) -> io::Result<()> {
        let bytes = consensus::serialize(val);
        BorshSerialize::serialize(&(bytes.len() as u32), writer)?;
        writer.write_all(&bytes)
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<TxOut> {
        let len = u32::deserialize_reader(reader)? as usize;
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        consensus::deserialize(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// Borsh helpers for [`bitcoin::ScriptBuf`] fields.
pub mod script_buf {
    use bitcoin::ScriptBuf;

    use super::*;

    pub fn serialize<W: Write>(val: &ScriptBuf, writer: &mut W) -> io::Result<()> {
        let bytes = val.to_bytes();
        BorshSerialize::serialize(&(bytes.len() as u32), writer)?;
        writer.write_all(&bytes)
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<ScriptBuf> {
        let len = u32::deserialize_reader(reader)? as usize;
        let mut bytes = vec![0u8; len];
        reader.read_exact(&mut bytes)?;
        Ok(ScriptBuf::from(bytes))
    }
}

borsh_vec_helper!(vec_script_buf, bitcoin::ScriptBuf, script_buf);

borsh_option_helper!(option_txid, bitcoin::Txid, txid);

#[cfg(test)]
mod tests {
    use bitcoin::{Amount, OutPoint, ScriptBuf, TxOut, Txid, hashes::Hash};

    use super::*;

    #[test]
    fn txid_roundtrip() {
        let original = Txid::from_byte_array([42u8; 32]);
        let mut buf = Vec::new();
        txid::serialize(&original, &mut buf).unwrap();
        let decoded = txid::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn vec_txid_roundtrip() {
        let original = vec![
            Txid::from_byte_array([1u8; 32]),
            Txid::from_byte_array([2u8; 32]),
        ];
        let mut buf = Vec::new();
        vec_txid::serialize(&original, &mut buf).unwrap();
        let decoded = vec_txid::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn outpoint_roundtrip() {
        let original = OutPoint::new(Txid::from_byte_array([7u8; 32]), 42);
        let mut buf = Vec::new();
        outpoint::serialize(&original, &mut buf).unwrap();
        let decoded = outpoint::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn txout_roundtrip() {
        let original = TxOut {
            value: Amount::from_sat(50_000),
            script_pubkey: ScriptBuf::from_bytes(vec![0x51, 0x21, 0xFF]),
        };
        let mut buf = Vec::new();
        txout::serialize(&original, &mut buf).unwrap();
        let decoded = txout::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn script_buf_roundtrip() {
        let original = ScriptBuf::from_bytes(vec![0x51, 0x21, 0xFF]);
        let mut buf = Vec::new();
        script_buf::serialize(&original, &mut buf).unwrap();
        let decoded = script_buf::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn script_buf_empty_roundtrip() {
        let original = ScriptBuf::new();
        let mut buf = Vec::new();
        script_buf::serialize(&original, &mut buf).unwrap();
        let decoded = script_buf::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn vec_script_buf_roundtrip() {
        let original = vec![
            ScriptBuf::from_bytes(vec![0x51]),
            ScriptBuf::new(),
            ScriptBuf::from_bytes(vec![0x21, 0xFF]),
        ];
        let mut buf = Vec::new();
        vec_script_buf::serialize(&original, &mut buf).unwrap();
        let decoded = vec_script_buf::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn option_txid_some_roundtrip() {
        let original = Some(Txid::from_byte_array([99u8; 32]));
        let mut buf = Vec::new();
        option_txid::serialize(&original, &mut buf).unwrap();
        let decoded = option_txid::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn option_txid_none_roundtrip() {
        let original: Option<Txid> = None;
        let mut buf = Vec::new();
        option_txid::serialize(&original, &mut buf).unwrap();
        let decoded = option_txid::deserialize(&mut &buf[..]).unwrap();
        assert_eq!(original, decoded);
    }
}
