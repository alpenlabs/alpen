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
        }

        impl Into<$target> for $inner {
            fn into(self) -> $target {
                <$target>::new(self)
            }
        }

        impl Into<$inner> for $target {
            fn into(self) -> $inner {
                self.0
            }
        }
    };
}
