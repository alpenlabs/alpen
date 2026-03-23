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
    };
}
