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
