/// Generates `Arbitrary` impl for property-based testing.
macro_rules! impl_buf_arbitrary {
    ($name:ident, $len:expr) => {
        impl<'a> ::arbitrary::Arbitrary<'a> for $name {
            fn arbitrary(u: &mut ::arbitrary::Unstructured<'a>) -> ::arbitrary::Result<Self> {
                let mut array = [0u8; $len];
                u.fill_buffer(&mut array)?;
                Ok(array.into())
            }
        }
    };
}

pub(crate) use impl_buf_arbitrary;
