/// Generates impls for shims wrapping a type as another.
///
/// This must be a newtype a la `struct Foo(Bar);`.
#[macro_export]
macro_rules! impl_thin_wrapper {
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
