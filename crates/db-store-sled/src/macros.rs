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
macro_rules! define_table_with_default_codec {
    ($(#[$docs:meta])+ ($table_name:ident) $key:ty => $value:ty) => {
        define_table_without_codec!($(#[$docs])+ ( $table_name ) $key => $value);

        impl ::typed_sled::codec::KeyCodec<$table_name> for $key {
            fn encode_key(&self) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                ::borsh::to_vec(self).map_err(Into::into)
            }

            fn decode_key(data: &[u8]) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                ::borsh::BorshDeserialize::deserialize_reader(&mut &data[..]).map_err(Into::into)
            }
        }

        impl_borsh_value_codec!($table_name, $value);
    };
}

/// Variation of [`define_table_with_default_codec`].
///
/// It is generally used for schemas with integer keys. [`typed_sled::codec::KeyCodec`] is
/// implemented for all the integer types and this macro leverages that.
#[macro_export]
macro_rules! define_table_with_integer_key {
    ($(#[$docs:meta])+ ($table_name:ident) $key:ty => $value:ty) => {
        define_table_without_codec!($(#[$docs])+ ( $table_name ) $key => $value);

        impl_borsh_value_codec!($table_name, $value);
    };
}

/// Variation of [`define_table_with_default_codec`].
///
/// It shall be used when your key type should be serialized lexicographically.
///
/// Borsh serializes integers as little-endian, but lexicographic
/// ordering requires big-endian for proper sorting, so we use [`bincode`]
/// with the big-endian option here. This ensures consistent key ordering
/// for range queries and seeks.
#[macro_export]
macro_rules! define_table_with_seek_key_codec {
    ($(#[$docs:meta])+ ($table_name:ident) $key:ty => $value:ty) => {
        define_table_without_codec!($(#[$docs])+ ( $table_name ) $key => $value);

        impl ::typed_sled::codec::KeyCodec<$table_name> for $key {
            fn encode_key(&self) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                use ::anyhow::Context as _;
                use ::bincode::Options as _;

                let bincode_options = ::bincode::options()
                    .with_fixint_encoding()
                    .with_big_endian();

                bincode_options.serialize(self).context("Failed to serialize key").map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }

            fn decode_key(data: &[u8]) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                use ::anyhow::Context as _;
                use ::bincode::Options as _;

                let bincode_options = ::bincode::options()
                    .with_fixint_encoding()
                    .with_big_endian();

                bincode_options.deserialize_from(&mut &data[..]).context("Failed to deserialize key").map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }
        }

        impl_borsh_value_codec!($table_name, $value);
    };
}

/// Variation of [`define_table_with_default_codec`].
///
/// It shall be used when your key type should be serialized lexicographically.
///
/// Borsh serializes integers as little-endian, but RocksDB uses lexicographic
/// ordering which is only compatible with big-endian, so we use [`bincode`]
/// with the big-endian option here.
#[macro_export]
macro_rules! impl_bincode_key_codec {
    ($table_name:ident, $key:ty) => {
        impl ::typed_sled::codec::KeyCodec<$table_name> for $key {
            fn encode_key(
                &self,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                use ::bincode::Options as _;

                let bincode_options = ::bincode::options()
                    .with_fixint_encoding()
                    .with_big_endian();

                bincode_options.serialize(self).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }

            fn decode_key(
                data: &[u8],
            ) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                use ::bincode::Options as _;

                let bincode_options = ::bincode::options()
                    .with_fixint_encoding()
                    .with_big_endian();

                bincode_options
                    .deserialize_from(&mut &data[..])
                    .map_err(|err| ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    })
            }
        }
    };
}

#[macro_export]
macro_rules! impl_borsh_value_codec {
    ($table_name:ident, $value:ty) => {
        impl ::typed_sled::codec::ValueCodec<$table_name> for $value {
            fn encode_value(
                &self,
            ) -> ::std::result::Result<::std::vec::Vec<u8>, ::typed_sled::codec::CodecError> {
                ::borsh::to_vec(self).map_err(|err| {
                    ::typed_sled::codec::CodecError::SerializationFailed {
                        schema: $table_name::tree_name(),
                        source: err.into(),
                    }
                })
            }

            fn decode_value(
                data: &[u8],
            ) -> ::std::result::Result<Self, ::typed_sled::codec::CodecError> {
                ::borsh::BorshDeserialize::deserialize_reader(&mut &data[..]).map_err(|err| {
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
            $($field:ident: $schema:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $db_name {
            $(
                $field: typed_sled::SledTree<$schema>,
            )*
            #[allow(dead_code, clippy::allow_attributes, reason = "some generated code is not used")]
            config: $crate::SledDbConfig,
        }

        impl $db_name {
            pub fn new(db: std::sync::Arc<typed_sled::SledDb>, config: $crate::SledDbConfig) -> strata_db::DbResult<Self> {
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
