#[macro_export]
macro_rules! define_table_without_codec {
    ($(#[$docs:meta])+ ( $table_name:ident ) $key:ty => $value:ty) => {
        $(#[$docs])+
        ///
        #[doc = concat!("Takes [`", stringify!($key), "`] as a key and returns [`", stringify!($value), "`]")]
        #[derive(Clone, Copy, Debug, Default)]
        pub(crate) struct $table_name;

        impl ::typed_sled::Schema for $table_name {
            const TREE_NAME: ::typed_sled::schema::TreeName = ::typed_sled::schema::TreeName($table_name::tree_name());
            type Key = $key;
            type Value = $value;
        }

        impl $table_name {
            const fn tree_name() -> &'static str {
                ::core::stringify!($table_name)
            }
        }

        impl ::std::fmt::Display for $table_name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::core::write!(f, "{}", stringify!($table_name))
            }
        }
    };
}

pub(crate) mod lexicographic {
    use anyhow::{Context, anyhow};
    use strata_primitives::{
        Buf32, EvmEeBlockCommitment, L1BlockCommitment, L1BlockId, L2BlockCommitment,
        OLBlockCommitment, OLBlockId,
        proof::{ProofContext, ProofKey, ProofZkVm},
    };

    /// Trait for types that can be encoded and decoded lexicographically.
    pub(crate) trait LexicographicKey: Sized {
        fn encode_lexicographic(&self, out: &mut Vec<u8>);
        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self>;
    }

    /// Encode a lexicographic key into bytes.
    pub(crate) fn encode_key<T: LexicographicKey>(value: &T) -> Vec<u8> {
        let mut out = Vec::new();
        value.encode_lexicographic(&mut out);
        out
    }

    /// Decode a lexicographic key from bytes.
    pub(crate) fn decode_key<T: LexicographicKey>(data: &[u8]) -> anyhow::Result<T> {
        let mut remaining = data;
        let value = T::decode_lexicographic(&mut remaining)?;
        if !remaining.is_empty() {
            return Err(anyhow!("lexicographic key has trailing bytes"));
        }
        Ok(value)
    }

    /// Read exactly `N` bytes from the data.
    fn read_exact<const N: usize>(data: &mut &[u8]) -> anyhow::Result<[u8; N]> {
        if data.len() < N {
            return Err(anyhow!(
                "lexicographic key underflow: need {N} bytes, got {}",
                data.len()
            ));
        }
        let (prefix, rest) = data.split_at(N);
        *data = rest;
        let mut out = [0u8; N];
        out.copy_from_slice(prefix);
        Ok(out)
    }

    /// Read a single byte from the data.
    fn read_u8(data: &mut &[u8]) -> anyhow::Result<u8> {
        Ok(read_exact::<1>(data)?[0])
    }

    /// Read an unsigned 32-bit integer from the data.
    fn read_u32(data: &mut &[u8]) -> anyhow::Result<u32> {
        Ok(u32::from_be_bytes(read_exact::<4>(data)?))
    }

    /// Read an unsigned 64-bit integer from the data.
    fn read_u64(data: &mut &[u8]) -> anyhow::Result<u64> {
        Ok(u64::from_be_bytes(read_exact::<8>(data)?))
    }

    /// Write a single byte to the output.
    fn write_u8(out: &mut Vec<u8>, value: u8) {
        out.push(value);
    }

    /// Write an unsigned 32-bit integer to the output.
    fn write_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    /// Write an unsigned 64-bit integer to the output.
    fn write_u64(out: &mut Vec<u8>, value: u64) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    impl LexicographicKey for u8 {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            write_u8(out, *self);
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            read_u8(data)
        }
    }

    impl LexicographicKey for u32 {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            write_u32(out, *self);
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            read_u32(data)
        }
    }

    impl LexicographicKey for u64 {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            write_u64(out, *self);
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            read_u64(data)
        }
    }

    impl LexicographicKey for Vec<u8> {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            let len = u32::try_from(self.len())
                .expect("lexicographic Vec<u8> length should fit into u32");
            write_u32(out, len);
            out.extend_from_slice(self);
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let len = read_u32(data)? as usize;
            if data.len() < len {
                return Err(anyhow!(
                    "lexicographic Vec<u8> length mismatch: expected {len}, got {}",
                    data.len()
                ));
            }
            let (value, rest) = data.split_at(len);
            *data = rest;
            Ok(value.to_vec())
        }
    }

    impl LexicographicKey for Buf32 {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(self.as_ref());
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let bytes = read_exact::<32>(data)?;
            Ok(Buf32::new(bytes))
        }
    }

    impl LexicographicKey for L1BlockId {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(self.as_ref());
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let bytes = read_exact::<32>(data)?;
            Ok(L1BlockId::from(Buf32::new(bytes)))
        }
    }

    impl LexicographicKey for OLBlockId {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(self.as_ref());
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let bytes = read_exact::<32>(data)?;
            Ok(OLBlockId::from(Buf32::new(bytes)))
        }
    }

    impl LexicographicKey for L1BlockCommitment {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            write_u64(out, self.height_u64());
            out.extend_from_slice(self.blkid().as_ref());
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let height = read_u64(data)?;
            let blkid = L1BlockId::decode_lexicographic(data)?;
            L1BlockCommitment::from_height_u64(height, blkid)
                .context("invalid L1BlockCommitment height")
        }
    }

    impl LexicographicKey for OLBlockCommitment {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            write_u64(out, self.slot());
            out.extend_from_slice(self.blkid().as_ref());
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let slot = read_u64(data)?;
            let blkid = OLBlockId::decode_lexicographic(data)?;
            Ok(Self::new(slot, blkid))
        }
    }

    impl LexicographicKey for EvmEeBlockCommitment {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            write_u64(out, self.slot());
            out.extend_from_slice(self.blkid().as_ref());
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let slot = read_u64(data)?;
            let blkid = Buf32::decode_lexicographic(data)?;
            Ok(Self::new(slot, blkid))
        }
    }

    impl LexicographicKey for ProofContext {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            match self {
                ProofContext::EvmEeStf(start, end) => {
                    write_u8(out, 0);
                    start.encode_lexicographic(out);
                    end.encode_lexicographic(out);
                }
                ProofContext::ClStf(start, end) => {
                    write_u8(out, 1);
                    start.encode_lexicographic(out);
                    end.encode_lexicographic(out);
                }
                ProofContext::Checkpoint(checkpoint) => {
                    write_u8(out, 2);
                    write_u64(out, *checkpoint);
                }
            }
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let tag = read_u8(data)?;
            match tag {
                0 => {
                    let start = EvmEeBlockCommitment::decode_lexicographic(data)?;
                    let end = EvmEeBlockCommitment::decode_lexicographic(data)?;
                    Ok(Self::EvmEeStf(start, end))
                }
                1 => {
                    let start = L2BlockCommitment::decode_lexicographic(data)?;
                    let end = L2BlockCommitment::decode_lexicographic(data)?;
                    Ok(Self::ClStf(start, end))
                }
                2 => Ok(Self::Checkpoint(read_u64(data)?)),
                _ => Err(anyhow!("unknown ProofContext tag {tag}")),
            }
        }
    }

    impl LexicographicKey for ProofZkVm {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            let tag = match self {
                ProofZkVm::SP1 => 0,
                ProofZkVm::Native => 1,
                _ => 255,
            };
            write_u8(out, tag);
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            match read_u8(data)? {
                0 => Ok(Self::SP1),
                1 => Ok(Self::Native),
                tag => Err(anyhow!("unknown ProofZkVm tag {tag}")),
            }
        }
    }

    impl LexicographicKey for ProofKey {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            self.context().encode_lexicographic(out);
            self.host().encode_lexicographic(out);
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let context = ProofContext::decode_lexicographic(data)?;
            let host = ProofZkVm::decode_lexicographic(data)?;
            Ok(Self::new(context, host))
        }
    }

    impl<A, B> LexicographicKey for (A, B)
    where
        A: LexicographicKey,
        B: LexicographicKey,
    {
        fn encode_lexicographic(&self, out: &mut Vec<u8>) {
            self.0.encode_lexicographic(out);
            self.1.encode_lexicographic(out);
        }

        fn decode_lexicographic(data: &mut &[u8]) -> anyhow::Result<Self> {
            let first = A::decode_lexicographic(data)?;
            let second = B::decode_lexicographic(data)?;
            Ok((first, second))
        }
    }
}

#[macro_export]
macro_rules! define_table_with_default_codec {
    ($(#[$docs:meta])+ ($table_name:ident) $key:ty => $value:ty) => {
        $crate::define_table_without_codec!($(#[$docs])+ ( $table_name ) $key => $value);

        $crate::impl_rkyv_key_codec!($table_name, $key);
        $crate::impl_rkyv_value_codec!($table_name, $value);
    };
}

/// Variation of [`define_table_with_default_codec`].
///
/// It is generally used for schemas with integer keys. [`typed_sled::codec::KeyCodec`] is
/// implemented for all the integer types and this macro leverages that.
#[macro_export]
macro_rules! define_table_with_integer_key {
    ($(#[$docs:meta])+ ($table_name:ident) $key:ty => $value:ty) => {
        $crate::define_table_without_codec!($(#[$docs])+ ( $table_name ) $key => $value);

        $crate::impl_rkyv_value_codec!($table_name, $value);
    };
}

/// Variation of [`define_table_with_default_codec`].
///
/// It shall be used when your key type should be serialized lexicographically.
///
/// Lexicographic ordering requires big-endian encoding for integer fields,
/// so we use a dedicated key codec that writes fixed-width big-endian values
/// and preserves ordering for range queries and seeks.
#[macro_export]
macro_rules! define_table_with_seek_key_codec {
    ($(#[$docs:meta])+ ($table_name:ident) $key:ty => $value:ty) => {
        $crate::define_table_without_codec!($(#[$docs])+ ( $table_name ) $key => $value);

        impl ::typed_sled::codec::KeyCodec<$table_name> for $key {
            fn encode_key(&self) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                Ok($crate::macros::lexicographic::encode_key(self))
            }

            fn decode_key(data: &[u8]) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                $crate::macros::lexicographic::decode_key(data).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }
        }

        $crate::impl_rkyv_value_codec!($table_name, $value);
    };
}

/// Implements the default rkyv key codec for a table.
#[macro_export]
macro_rules! impl_rkyv_key_codec {
    ($table_name:ident, $key:ty) => {
        impl ::typed_sled::codec::KeyCodec<$table_name> for $key {
            fn encode_key(
                &self,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                ::rkyv::to_bytes::<::rkyv::rancor::Error>(self)
                    .map(|bytes| bytes.as_ref().to_vec())
                    .map_err(|err| ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    })
            }

            fn decode_key(
                data: &[u8],
            ) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                let mut aligned =
                    ::rkyv::util::AlignedVec::<{ ::std::mem::align_of::<$key>() }>::with_capacity(
                        data.len(),
                    );
                aligned.extend_from_slice(data);
                ::rkyv::from_bytes::<$key, ::rkyv::rancor::Error>(&aligned).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }
        }
    };
}

#[macro_export]
macro_rules! impl_rkyv_value_codec {
    ($table_name:ident, $value:ty) => {
        impl ::typed_sled::codec::ValueCodec<$table_name> for $value {
            fn encode_value(
                &self,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                ::rkyv::to_bytes::<::rkyv::rancor::Error>(self)
                    .map(|bytes| bytes.as_ref().to_vec())
                    .map_err(|err| ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    })
            }

            fn decode_value(
                data: &[u8],
            ) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                let mut aligned =
                    ::rkyv::util::AlignedVec::<{ ::std::mem::align_of::<$value>() }>::with_capacity(
                        data.len(),
                    );
                aligned.extend_from_slice(data);
                ::rkyv::from_bytes::<$value, ::rkyv::rancor::Error>(&aligned).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }
        }
    };
}

#[macro_export]
macro_rules! impl_bytes_value_codec {
    ($table_name:ident) => {
        impl ::typed_sled::codec::ValueCodec<$table_name> for ::std::vec::Vec<u8> {
            fn encode_value(
                &self,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                Ok(self.clone())
            }

            fn decode_value(
                data: &[u8],
            ) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                Ok(data.to_vec())
            }
        }
    };
}

#[macro_export]
macro_rules! impl_integer_value_codec {
    ($table_name:ident, $int:ty) => {
        impl ::typed_sled::codec::ValueCodec<$table_name> for $int {
            fn encode_value(
                &self,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                Ok(self.to_be_bytes().into())
            }

            fn decode_value(
                buf: &[u8],
            ) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                const SIZE: usize = ::std::mem::size_of::<$int>();
                if buf.len() != SIZE {
                    return Err(::typed_sled::codec::CodecError::Other(format!(
                        "invalid value length in '{}' (expected {} bytes, got {})",
                        $table_name::tree_name(),
                        SIZE,
                        buf.len()
                    )));
                }
                let mut bytes = [0u8; SIZE];
                bytes.copy_from_slice(buf);
                Ok(<$int>::from_be_bytes(bytes))
            }
        }
    };
}

#[macro_export]
macro_rules! impl_codec_key_codec {
    ($table_name:ident, $key:ty) => {
        impl ::typed_sled::codec::KeyCodec<$table_name> for $key {
            fn encode_key(
                &self,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                ::strata_codec::encode_to_vec(self).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }

            fn decode_key(
                data: &[u8],
            ) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                use ::strata_codec::{BufDecoder, Codec};
                let mut decoder = BufDecoder::new(data);
                Codec::decode(&mut decoder).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }
        }
    };
}

#[macro_export]
macro_rules! impl_codec_value_codec {
    ($table_name:ident, $value:ty) => {
        impl ::typed_sled::codec::ValueCodec<$table_name> for $value {
            fn encode_value(
                &self,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                ::strata_codec::encode_to_vec(self).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }

            fn decode_value(
                data: &[u8],
            ) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                use ::strata_codec::{BufDecoder, Codec};
                let mut decoder = BufDecoder::new(data);
                Codec::decode(&mut decoder).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }
        }
    };
}

#[macro_export]
macro_rules! sled_db_test_setup {
    ($db_type:ty, $test_macro:ident) => {
        fn setup_db() -> $db_type {
            let db = sled::Config::new().temporary(true).open().unwrap();
            let sled_db = typed_sled::SledDb::new(db).unwrap();
            let config = $crate::SledDbConfig::test();
            <$db_type>::new(sled_db.into(), config).unwrap()
        }

        $test_macro!(setup_db());
    };
}

#[macro_export]
macro_rules! define_sled_database {
    (
        $(#[$meta:meta])*
        pub struct $db_name:ident {
            $($vis:vis $field:ident: $schema:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $db_name {
            $(
                $vis $field: typed_sled::SledTree<$schema>,
            )*
            #[allow(dead_code, clippy::allow_attributes, reason = "some generated code is not used")]
            config: $crate::SledDbConfig,
        }

        impl $db_name {
            pub fn new(db: std::sync::Arc<typed_sled::SledDb>, config: $crate::SledDbConfig) -> strata_db_types::DbResult<Self> {
                Ok(Self {
                    $(
                        $field: db.get_tree()?,
                    )*
                    config,
                })
            }
        }
    };
}
