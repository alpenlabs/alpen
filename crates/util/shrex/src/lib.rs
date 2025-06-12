//! A module for encoding and decoding hexadecimal strings using bitwise ops.

use std::fmt::Debug;
#[cfg(any(feature = "serde", feature = "axum"))]
use std::ops::{Deref, DerefMut};

#[cfg(feature = "axum")]
use axum::{body::Body, http::Response, response::IntoResponse};
#[cfg(feature = "serde")]
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use terrors::OneOf;

macro_rules! err {
    ($item:expr) => {{
        ::core::result::Result::Err(::terrors::OneOf::new($item))
    }};
}

#[macro_export]
macro_rules! hex {
    ($hex_literal:expr) => {{
        const HEX_LITERAL: &str = $hex_literal;

        // Calculate the length of the resulting array
        const BUF_LEN: usize = HEX_LITERAL.len() / 2;

        // Create a buffer with the correct length
        let mut buf = [0u8; BUF_LEN];

        // Decode the hex string into the buffer
        match ::shrex::decode(HEX_LITERAL, &mut buf) {
            Ok(_) => buf,
            Err(e) => panic!("Failed to decode hex literal: {:?}", e),
        }
    }};
}

#[cfg(any(feature = "serde", feature = "axum"))]
pub struct Hex<T: AsRef<[u8]>>(pub T);

#[cfg(feature = "axum")]
impl<T: AsRef<[u8]>> IntoResponse for Hex<T> {
    fn into_response(self) -> axum::response::Response {
        Response::builder()
            .body(Body::from(encode(self.0.as_ref())))
            .expect("valid response")
    }
}

#[cfg(feature = "serde")]
impl<T: AsRef<[u8]>> Serialize for Hex<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_string = encode(self.0.as_ref());
        serializer.serialize_str(&hex_string)
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for Hex<Vec<u8>> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_string = String::deserialize(deserializer)?;
        let bytes = decode_alloc(&hex_string).map_err(|e| de::Error::custom(format!("{e:?}")))?;
        Ok(Hex(bytes))
    }
}

#[cfg(feature = "serde")]
impl<'de, const N: usize> Deserialize<'de> for Hex<[u8; N]> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_string = String::deserialize(deserializer)?;
        let mut buf = [0; N];
        decode(&hex_string, &mut buf).map_err(|e| de::Error::custom(format!("{e:?}")))?;
        Ok(Hex(buf))
    }
}

#[cfg(any(feature = "serde", feature = "axum"))]
impl<T: AsRef<[u8]>> std::fmt::Debug for Hex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&encode(self.0.as_ref()))
    }
}

#[cfg(any(feature = "serde", feature = "axum"))]
impl<T: AsRef<[u8]>> Deref for Hex<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(any(feature = "serde", feature = "axum"))]
impl<T: AsRef<[u8]>> DerefMut for Hex<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub fn encode(bytes: &[u8]) -> String {
    let mut hex_string = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        hex_string.push(
            // first char represents first 4 bites of the byte, so get
            // the hex char for that 4 bits by shifting it right
            HexChar::try_from(b >> 4)
                .expect("value should be 0..=15")
                .into(),
        );
        hex_string.push(
            // for the second char, the bits are already in the correct position
            // but the first char is still there, so AND the first 4 bits with 0
            // to set them all to 0
            HexChar::try_from(b & 0b00001111)
                .expect("value should be 0..=15")
                .into(),
        );
    }
    hex_string
}

pub fn decode_alloc(hex: &str) -> Result<Vec<u8>, OneOf<(BadByte, UnevenHexCharacterCount)>> {
    let mut buf = vec![0u8; hex.len() / 2];
    decode(hex, &mut buf).map_err(|e| e.subset().expect("should not be WrongBufLength"))?;
    Ok(buf)
}

pub fn decode(
    hex: &str,
    buf: &mut [u8],
) -> Result<(), OneOf<(WrongBufLength, BadByte, UnevenHexCharacterCount)>> {
    let half_hex_len = hex.len() / 2;
    if half_hex_len * 2 != hex.len() {
        return err!(UnevenHexCharacterCount);
    }
    if buf.len() < half_hex_len {
        return err!(WrongBufLength {
            needed: half_hex_len,
            got: buf.len()
        });
    }

    let mut chars = hex.chars();

    let mut byte_count = 0;
    loop {
        let (c1, c2) = match (chars.next(), chars.next()) {
            (Some(c1), Some(c2)) => (c1, c2),
            (None, None) => break Ok(()),
            _ => break err!(BadByte { byte: byte_count }),
        };

        let (Ok(c1), Ok(c2)) = (HexChar::try_from(c1), HexChar::try_from(c2)) else {
            for byte in buf {
                *byte = 0;
            }
            return err!(BadByte { byte: byte_count });
        };

        // first character is the first 4 bits so we calculate its
        // value then shift left by 4 bits.
        // we then calculate the second character by correcting its offset then
        // ORing the two together for the last 4 bits from c2
        buf[byte_count] = (<HexChar as Into<u8>>::into(c1) << 4) | <HexChar as Into<u8>>::into(c2);
        byte_count += 1;
    }
}

#[derive(Debug)]
pub struct WrongBufLength {
    pub needed: usize,
    pub got: usize,
}

#[derive(Debug)]
pub struct BadByte {
    #[allow(dead_code)]
    byte: usize,
}

#[derive(Debug)]
pub struct UnevenHexCharacterCount;

enum HexCharKind {
    Uppercase,
    Lowercase,
    Number,
}

impl TryFrom<&char> for HexCharKind {
    type Error = ();

    fn try_from(value: &char) -> Result<Self, Self::Error> {
        Ok(match value {
            '0'..='9' => Self::Number,
            'a'..='f' => Self::Lowercase,
            'A'..='F' => Self::Uppercase,
            _ => Err(())?,
        })
    }
}

impl TryFrom<&u8> for HexCharKind {
    type Error = ();

    fn try_from(value: &u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0..=9 => Self::Number,
            10..=15 => Self::Lowercase,
            _ => Err(())?,
        })
    }
}

impl HexCharKind {
    fn offset(&self) -> u8 {
        match self {
            HexCharKind::Uppercase => b'A' - 0xA,
            HexCharKind::Lowercase => b'a' - 0xa,
            HexCharKind::Number => b'0',
        }
    }
}

struct HexChar {
    c: char,
    kind: HexCharKind,
}

impl TryFrom<char> for HexChar {
    type Error = ();

    fn try_from(value: char) -> Result<Self, Self::Error> {
        Ok(Self {
            kind: (&value).try_into()?,
            c: value,
        })
    }
}

impl TryFrom<u8> for HexChar {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        let kind = HexCharKind::try_from(&value)?;
        Ok(Self {
            c: (value + kind.offset()) as char,
            kind,
        })
    }
}

impl From<HexChar> for u8 {
    /// Converts the character to its [`u8`] equivalent. Eg, 'a' becomes 10.
    ///
    /// SAFETY: self is checked to be a valid hex character by `HexChar`'s
    /// [`TryFrom`] implementation.
    fn from(val: HexChar) -> Self {
        val.c as u8 - val.kind.offset()
    }
}

impl From<HexChar> for char {
    fn from(val: HexChar) -> Self {
        val.c
    }
}

#[cfg(test)]
mod tests {
    use rand::{rngs::OsRng, Rng};

    use super::*;

    #[test]
    fn hex_char_from_char() {
        for range in ['0'..='9', 'a'..='f', 'A'..='F'] {
            for c in range {
                assert!(HexChar::try_from(c).is_ok())
            }
        }
        for c in ['g', 'j', '.', '/'] {
            assert!(HexChar::try_from(c).is_err())
        }
    }

    #[test]
    fn hex_char_from_u8() {
        for u in 0u8..16 {
            assert!(HexChar::try_from(u).is_ok())
        }

        for u in 16u8..100 {
            assert!(HexChar::try_from(u).is_err())
        }
    }

    #[test]
    fn e2e() {
        let buf: [u8; 32] = OsRng.gen();
        let string = encode(&buf);
        let mut debuf = [0u8; 32];
        assert!(decode(&string, &mut debuf).is_ok());
        assert_eq!(buf, debuf);

        let allocd = decode_alloc(&string);
        assert!(allocd.is_ok());
        if let Ok(vec) = allocd {
            assert_eq!(vec.len(), buf.len());
            for i in 0..buf.len() {
                assert_eq!(vec[i], buf[i])
            }
        }
    }
}
