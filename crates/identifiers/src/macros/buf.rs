/// Generates the foundational API for a fixed-size byte buffer type.
///
/// Provides constructors (`new`, `zero`), accessors (`as_slice`, `as_mut_slice`,
/// `as_bytes`, `is_zero`), the `LEN` constant, standard conversion traits (`AsRef`,
/// `AsMut`, `From`, `TryFrom`), and `Default`.
macro_rules! impl_buf_core {
    ($name:ident, $len:expr) => {
        impl $name {
            pub const LEN: usize = $len;

            pub const fn new(data: [u8; $len]) -> Self {
                Self(data)
            }

            pub const fn as_slice(&self) -> &[u8] {
                &self.0
            }

            pub const fn as_mut_slice(&mut self) -> &mut [u8] {
                &mut self.0
            }

            pub const fn as_bytes(&self) -> &[u8] {
                self.0.as_slice()
            }

            pub const fn zero() -> Self {
                Self::new([0; $len])
            }

            pub const fn is_zero(&self) -> bool {
                let mut i = 0;
                while i < $len {
                    if self.0[i] != 0 {
                        return false;
                    }
                    i += 1;
                }
                true
            }
        }

        impl ::std::convert::AsRef<[u8; $len]> for $name {
            fn as_ref(&self) -> &[u8; $len] {
                &self.0
            }
        }

        impl ::std::convert::AsMut<[u8]> for $name {
            fn as_mut(&mut self) -> &mut [u8] {
                &mut self.0
            }
        }

        impl ::std::convert::From<[u8; $len]> for $name {
            fn from(data: [u8; $len]) -> Self {
                Self(data)
            }
        }

        impl ::std::convert::From<$name> for [u8; $len] {
            fn from(buf: $name) -> Self {
                buf.0
            }
        }

        impl<'a> ::std::convert::From<&'a [u8; $len]> for $name {
            fn from(data: &'a [u8; $len]) -> Self {
                Self(*data)
            }
        }

        impl<'a> ::std::convert::TryFrom<&'a [u8]> for $name {
            type Error = &'a [u8];

            fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
                if value.len() == $len {
                    let mut arr = [0; $len];
                    arr.copy_from_slice(value);
                    Ok(Self(arr))
                } else {
                    Err(value)
                }
            }
        }

        impl ::std::default::Default for $name {
            fn default() -> Self {
                Self([0; $len])
            }
        }
    };
}

/// Generates `Debug` (full hex), `Display` (truncated hex), `FromStr` (hex parsing),
/// `LowerHex`, and `UpperHex` formatting.
macro_rules! impl_buf_fmt {
    ($name:ident, $len:expr) => {
        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Display::fmt(&::const_hex::display(&self.0), f)
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(
                    f,
                    "{}..{}",
                    ::const_hex::display(&self.0[..3]),
                    ::const_hex::display(&self.0[$len - 3..]),
                )
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = ::const_hex::FromHexError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                ::const_hex::decode_to_array(s).map(Self::new)
            }
        }

        impl ::std::fmt::LowerHex for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::LowerHex::fmt(&::const_hex::display(&self.0), f)
            }
        }

        impl ::std::fmt::UpperHex for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::UpperHex::fmt(&::const_hex::display(&self.0), f)
            }
        }
    };
}

/// Generates `Debug` (full reversed hex) and `Display` (truncated reversed hex) formatting.
///
/// Same as [`impl_buf_fmt`] but reverses the byte order before hex encoding,
/// matching Bitcoin's display convention for block/transaction hashes.
macro_rules! impl_rbuf_fmt {
    ($name:ident, $len:expr) => {
        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let mut bytes = self.0;
                bytes.reverse();
                ::std::fmt::Display::fmt(&::const_hex::display(&bytes), f)
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let mut bytes = self.0;
                bytes.reverse();
                write!(
                    f,
                    "{}..{}",
                    ::const_hex::display(&bytes[..3]),
                    ::const_hex::display(&bytes[$len - 3..]),
                )
            }
        }
    };
}

pub(crate) use impl_buf_core;
pub(crate) use impl_buf_fmt;
pub(crate) use impl_rbuf_fmt;
