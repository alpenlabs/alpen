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

#[macro_export]
macro_rules! impl_ssz_key_codec {
    ($schema:ty, $key:ty) => {
        impl ::typed_sled::codec::KeyCodec<$schema> for $key {
            fn encode_key(&self) -> Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                Ok(<$key as ::ssz::Encode>::as_ssz_bytes(self))
            }

            fn decode_key(data: &[u8]) -> Result<Self, ::typed_sled::codec::CodecError> {
                <$key as ::ssz::Decode>::from_ssz_bytes(data).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: <$schema>::tree_name(),
                        source: format!("SSZ decode error: {err:?}").into(),
                    }
                })
            }
        }
    };
}

#[macro_export]
macro_rules! impl_ssz_value_codec {
    ($schema:ty, $value:ty) => {
        impl ::typed_sled::codec::ValueCodec<$schema> for $value {
            fn encode_value(&self) -> Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                Ok(<$value as ::ssz::Encode>::as_ssz_bytes(self))
            }

            fn decode_value(data: &[u8]) -> Result<Self, ::typed_sled::codec::CodecError> {
                <$value as ::ssz::Decode>::from_ssz_bytes(data).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: <$schema>::tree_name(),
                        source: format!("SSZ decode error: {err:?}").into(),
                    }
                })
            }
        }
    };
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
                Ok($crate::lexicographic::encode_key(self))
            }

            fn decode_key(data: &[u8]) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                $crate::lexicographic::decode_key(data).map_err(|err| {
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
