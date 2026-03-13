/// Generates impls for shims wrapping a type as another.
///
/// This must be a newtype a la `struct Foo(Bar);`.
#[macro_export]
macro_rules! impl_opaque_thin_wrapper {
    ($target:ty => $inner:ty) => {
        impl $target {
            pub const fn new(v: $inner) -> Self {
                Self(v)
            }

            pub fn inner(&self) -> &$inner {
                &self.0
            }

            pub fn into_inner(self) -> $inner {
                self.0
            }
        }

        $crate::strata_codec::impl_wrapper_codec!($target => $inner);

        impl From<$inner> for $target {
            fn from(value: $inner) -> $target {
                <$target>::new(value)
            }
        }

        impl From<$target> for $inner {
            fn from(value: $target) -> $inner {
                value.into_inner()
            }
        }
    };
}

/// Generates impls for shims wrapping a type as another, but where this is a
/// transparent relationship.
///
/// This must be a newtype a la `struct Foo(Bar);`.
#[macro_export]
macro_rules! impl_transparent_thin_wrapper {
    ($target:ty => $inner:ty) => {
        $crate::impl_opaque_thin_wrapper! { $target => $inner }

        impl std::ops::Deref for $target {
            type Target = $inner;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::ops::DerefMut for $target {
            fn deref_mut(&mut self) -> &mut $inner {
                &mut self.0
            }
        }
    };
}

#[macro_export]
macro_rules! impl_buf_wrapper {
    ($wrapper:ident, $name:ident, $len:expr) => {
        impl ::std::convert::From<$name> for $wrapper {
            fn from(value: $name) -> Self {
                Self(value)
            }
        }

        impl ::std::convert::From<$wrapper> for $name {
            fn from(value: $wrapper) -> Self {
                value.0
            }
        }

        impl ::std::convert::AsRef<[u8; $len]> for $wrapper {
            fn as_ref(&self) -> &[u8; $len] {
                self.0.as_ref()
            }
        }

        impl ::core::fmt::Debug for $wrapper {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Debug::fmt(&self.0, f)
            }
        }

        impl ::core::fmt::Display for $wrapper {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }

        // Codec implementation for Buf wrapper types - passthrough to underlying Buf
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
