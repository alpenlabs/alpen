/// Generates impls for shims wrapping a type as another.
///
/// This must be a newtype a la `struct Foo(Bar);`.
#[macro_export]
macro_rules! impl_opaque_thin_wrapper {
    ($target:ty => $inner:ty) => {
        impl $target {
            pub fn new(v: $inner) -> Self {
                Self(v)
            }

            pub fn inner(&self) -> &$inner {
                &self.0
            }

            fn into_inner(self) -> $inner {
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
