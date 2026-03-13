/// Generates `strata_codec::Codec` impl for a fixed-size byte buffer.
macro_rules! impl_buf_codec {
    ($name:ident, $len:expr) => {
        impl $crate::strata_codec::Codec for $name {
            fn encode(
                &self,
                enc: &mut impl $crate::strata_codec::Encoder,
            ) -> Result<(), $crate::strata_codec::CodecError> {
                self.0.encode(enc)
            }

            fn decode(
                dec: &mut impl $crate::strata_codec::Decoder,
            ) -> Result<Self, $crate::strata_codec::CodecError> {
                let bytes = <[u8; $len]>::decode(dec)?;
                Ok(Self(bytes))
            }
        }
    };
}

/// Generates `strata_codec::Codec` wrapper delegation for opaque thin wrappers.
///
/// This is the codec counterpart to [`impl_opaque_thin_wrapper`], extracted so
/// that codec support can be gated behind a feature flag.
#[macro_export]
macro_rules! impl_opaque_thin_wrapper_codec {
    ($target:ty => $inner:ty) => {
        $crate::strata_codec::impl_wrapper_codec!($target => $inner);
    };
}

/// Generates `strata_codec::Codec` impl for buf wrapper types.
///
/// This is the codec counterpart to [`impl_buf_wrapper`], extracted so that
/// codec support can be gated behind a feature flag.
#[macro_export]
macro_rules! impl_buf_wrapper_codec {
    ($wrapper:ident, $name:ident, $len:expr) => {
        impl $crate::strata_codec::Codec for $wrapper {
            fn encode(
                &self,
                enc: &mut impl $crate::strata_codec::Encoder,
            ) -> Result<(), $crate::strata_codec::CodecError> {
                // Delegate to the underlying Buf type's Codec implementation
                self.0.encode(enc)
            }

            fn decode(
                dec: &mut impl $crate::strata_codec::Decoder,
            ) -> Result<Self, $crate::strata_codec::CodecError> {
                // Decode the underlying Buf type and wrap it
                let buf = $name::decode(dec)?;
                Ok(Self(buf))
            }
        }
    };
}

pub(crate) use impl_buf_codec;
