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
    };
}
