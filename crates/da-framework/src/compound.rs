//! Compound DA type infra.

use crate::{Codec, CodecResult, DaCounter, DaRegister, DaWrite, Decoder, Encoder};

/// Describes a bitmap we can read/write to.
pub trait Bitmap: Copy {
    /// Returns the total number of bits we can store.
    const BITS: u8;

    /// Returns an empty bitmap.
    fn zero() -> Self;

    /// Reads the bit at some some index.
    fn get(&self, off: u8) -> bool;

    /// Writes the bit at some index.
    fn put(&mut self, off: u8, b: bool);
}

macro_rules! impl_uint_bitmap {
    ($t:ident) => {
        impl Bitmap for $t {
            const BITS: u8 = $t::BITS as u8;

            fn zero() -> Self {
                0
            }

            fn get(&self, off: u8) -> bool {
                (*self >> off) & 1 == 1
            }

            fn put(&mut self, off: u8, b: bool) {
                let mask = 1 << off;
                if b {
                    *self |= mask;
                } else {
                    *self &= !mask;
                }
            }
        }
    };
}

impl_uint_bitmap!(u8);
impl_uint_bitmap!(u16);
impl_uint_bitmap!(u32);
impl_uint_bitmap!(u64);

/// Safer sequence interface around a [`Bitmap`] that ensures we don't overflow.
pub struct BitSeqReader<T: Bitmap> {
    off: u8,
    mask: T,
}

impl<T: Bitmap> BitSeqReader<T> {
    pub fn from_mask(v: T) -> Self {
        Self { off: 0, mask: v }
    }

    /// Returns the next bit, if possible.
    pub fn next(&mut self) -> bool {
        if self.off >= T::BITS {
            panic!("bitqueue: out of bits");
        }

        let b = self.mask.get(self.off);
        self.off += 1;
        b
    }

    /// Decodes a member of a compound, using the "default" value if the next
    /// bit is unset.
    pub fn decode_next_member<C: CompoundMember>(
        &mut self,
        dec: &mut impl Decoder,
    ) -> CodecResult<C> {
        let set = self.next();
        if set {
            C::decode_set(dec)
        } else {
            Ok(C::default())
        }
    }
}

/// Safer sequence interface around a [`Bitmap`] that ensures we don't overflow.
pub struct BitSeqWriter<T: Bitmap> {
    off: u8,
    mask: T,
}

impl<T: Bitmap> BitSeqWriter<T> {
    pub fn new() -> Self {
        Self {
            off: 0,
            mask: T::zero(),
        }
    }

    /// Prepares to write a compound member.
    pub fn prepare_member<C: CompoundMember>(&mut self, c: &C) {
        let b = !c.is_default();
        self.mask.put(self.off, b);
        self.off += 1;
    }

    pub fn mask(&self) -> T {
        self.mask
    }
}

/// Macro to generate encode/decode and apply impls for a compound DA type.
///
/// # Basic syntax (no type coercion)
///
/// Type specs must be wrapped in parentheses, braces, or brackets to form a single token tree.
///
/// ```ignore
/// make_compound_impl! {
///     DiffType u8 => TargetType {
///         field1: register (InnerType),
///         field2: counter (CounterScheme),
///     }
/// }
/// ```
///
/// # With type coercion
///
/// Use `[InnerType => TargetFieldType]` to specify that the target field has a different
/// type than the DA primitive's inner type. The inner type must implement `Into<TargetFieldType>`.
///
/// ```ignore
/// make_compound_impl! {
///     AccountDiff u8 => AccountSnapshot {
///         balance: register [CodecU256 => U256],  // CodecU256 converts to U256
///         nonce: counter (CtrU64ByU8),            // No coercion needed
///         code_hash: register [CodecB256 => B256],
///     }
/// }
/// ```
// TODO turn this into a proc macro
#[macro_export]
macro_rules! make_compound_impl {
    // Entry point without context type - delegates to version with () context
    (
        $tyname:ident $maskty:ident => $target:ty {
            $( $fname:ident : $daty:ident $fspec:tt ),* $(,)?
        }
    ) => {
        $crate::make_compound_impl! {
            $tyname < () > $maskty => $target {
                $( $fname : $daty $fspec ),*
            }
        }
    };

    // Main implementation with context type
    (
        $tyname:ident < $ctxty:ty > $maskty:ident => $target:ty {
            $( $fname:ident : $daty:ident $fspec:tt ),* $(,)?
        }
    ) => {
        // Compile-time check: ensure bitmap has enough bits for all fields.
        // Equal to const_assert! from static_assertions, but doesn't bring the dependency.
        const _: () = {
            const FIELD_COUNT: usize = [$(stringify!($fname)),*].len();
            const MASK_BITS: usize = <$maskty>::BITS as usize;
            assert!(FIELD_COUNT <= MASK_BITS, "compound type has more fields than bitmap can hold");
        };

        impl $crate::Codec for $tyname {
            fn decode(dec: &mut impl $crate::Decoder) -> Result<Self, $crate::CodecError> {
                let mask = <$maskty>::decode(dec)?;
                let mut bitr = $crate::compound::BitSeqReader::from_mask(mask);

                $(let $fname = $crate::_mct_field_decode!(bitr dec; $daty $fspec);)*

                Ok(Self { $($fname,)* })
            }

            fn encode(&self, enc: &mut impl $crate::Encoder) -> Result<(), $crate::CodecError> {
                let mut bitw = $crate::compound::BitSeqWriter::<$maskty>::new();

                $(bitw.prepare_member(&self.$fname);)*

                bitw.mask().encode(enc)?;

                // This goes through them in the same order as the above, which
                // is why this is safe.
                $(
                    if !$crate::CompoundMember::is_default(&self.$fname) {
                        $crate::CompoundMember::encode_set(&self.$fname, enc)?;
                    }
                )*

                Ok(())
            }
        }

        impl $crate::DaWrite for $tyname {
            type Target = $target;

            type Context = $ctxty;

            fn is_default(&self) -> bool {
                let mut v = true;

                // Kinda weird way to && all these different values.
                $(
                    v &= $crate::DaWrite::is_default(&self.$fname);
                )*

                v
            }

            fn poll_context(&self, _target: &Self::Target, _context: &Self::Context) -> Result<(), $crate::DaError> {
                // Note: poll_context is skipped for coercion fields since the types don't match.
                // This is fine since poll_context is mainly used for context validation.
                Ok(())
            }

            fn apply(&self, target: &mut Self::Target, _context: &Self::Context) -> Result<(), $crate::DaError> {
                // Depends on all the members being accessible.
                $(
                    $crate::_mct_field_apply!(self target; $fname $daty $fspec);
                )*
                Ok(())
            }
        }
    };
}

/// Expands to a decoder for each type of member that we support in a compound.
#[macro_export]
macro_rules! _mct_field_decode {
    // Register with coercion (decode uses inner type, coercion happens at apply)
    ($reader:ident $dec:ident; register [ $fty:ty => $targetfty:ty ]) => {
        $reader.decode_next_member::<$crate::DaRegister<$fty>>($dec)?
    };
    // Register without coercion - type is wrapped in parens or braces to be a single tt
    ($reader:ident $dec:ident; register ( $fty:ty )) => {
        $reader.decode_next_member::<$crate::DaRegister<$fty>>($dec)?
    };
    ($reader:ident $dec:ident; register { $fty:ty }) => {
        $reader.decode_next_member::<$crate::DaRegister<$fty>>($dec)?
    };
    // Counter - type is wrapped in parens or braces to be a single tt
    ($reader:ident $dec:ident; counter ( $fty:ty )) => {
        $reader.decode_next_member::<$crate::DaCounter<$fty>>($dec)?
    };
    ($reader:ident $dec:ident; counter { $fty:ty }) => {
        $reader.decode_next_member::<$crate::DaCounter<$fty>>($dec)?
    };
}

/// Expands to apply logic for each type of member that we support in a compound.
#[macro_export]
macro_rules! _mct_field_apply {
    // Register with coercion - use apply_into
    ($self:ident $target:ident; $fname:ident register [ $fty:ty => $targetfty:ty ]) => {
        $self.$fname.apply_into(&mut $target.$fname)
    };
    // Register without coercion - use standard DaWrite::apply
    ($self:ident $target:ident; $fname:ident register ( $fty:ty )) => {
        $crate::ContextlessDaWrite::apply(&$self.$fname, &mut $target.$fname)?
    };
    ($self:ident $target:ident; $fname:ident register { $fty:ty }) => {
        $crate::ContextlessDaWrite::apply(&$self.$fname, &mut $target.$fname)?
    };
    // Counter - use standard DaWrite::apply (counter targets scheme's Base type)
    ($self:ident $target:ident; $fname:ident counter ( $fty:ty )) => {
        $crate::ContextlessDaWrite::apply(&$self.$fname, &mut $target.$fname)?
    };
    ($self:ident $target:ident; $fname:ident counter { $fty:ty }) => {
        $crate::ContextlessDaWrite::apply(&$self.$fname, &mut $target.$fname)?
    };
}

/// Describes a member of a compound DA type.
///
/// This is necessary because we want to consolidate tagging across multiple
/// fields.
pub trait CompoundMember: Sized {
    /// Returns the default value.
    fn default() -> Self;

    /// Returns if this is a default value, and therefore shouldn't be encoded.
    fn is_default(&self) -> bool;

    /// Decodes a set value, since we know it to be in the modifying case.
    ///
    /// Returns an instance that we're setting.
    fn decode_set(dec: &mut impl Decoder) -> CodecResult<Self>;

    /// Encodes the new value, which we assume is in a modifying case.  This
    /// should be free of any tagging to indicate if the value is set or not, in
    /// this context we assume it's set.
    ///
    /// Returns error if actually unset.
    fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()>;
}

#[cfg(test)]
mod tests {
    use crate::{ContextlessDaWrite, DaRegister, encode_to_vec};

    #[derive(Copy, Clone, Eq, PartialEq, Debug)]
    pub struct Point {
        x: i32,
        y: i32,
    }

    #[derive(Debug, Default)]
    pub struct DaPointDiff {
        x: DaRegister<i32>,
        y: DaRegister<i32>,
    }

    make_compound_impl! {
        DaPointDiff u16 => Point {
            x: register (i32),
            y: register (i32),
        }
    }

    #[test]
    fn test_encoding_simple() {
        let p12 = DaPointDiff {
            x: DaRegister::new_unset(),
            y: DaRegister::new_set(32),
        };

        let p13 = DaPointDiff {
            x: DaRegister::new_set(8),
            y: DaRegister::new_unset(),
        };

        let p23 = DaPointDiff {
            x: DaRegister::new_set(8),
            y: DaRegister::new_set(16),
        };

        let buf12 = encode_to_vec(&p12).expect("test: encode p12");
        eprintln!("p12 {p12:?} buf12 {buf12:?}");
        assert_eq!(buf12, [0, 2, 0, 0, 0, 32]);

        let buf13 = encode_to_vec(&p13).expect("test: encode p13");
        eprintln!("p13 {p13:?} buf13 {buf13:?}");
        assert_eq!(buf13, [0, 1, 0, 0, 0, 8]);

        let buf23 = encode_to_vec(&p23).expect("test: encode p23");
        eprintln!("p23 {p23:?} buf23 {buf23:?}");
        assert_eq!(buf23, [0, 3, 0, 0, 0, 8, 0, 0, 0, 16]);
    }

    #[test]
    fn test_apply_simple() {
        let p1 = Point { x: 2, y: 16 };
        let p2 = Point { x: 2, y: 32 };
        let p3 = Point { x: 8, y: 16 };

        let p12 = DaPointDiff {
            x: DaRegister::new_unset(),
            y: DaRegister::new_set(32),
        };

        let p13 = DaPointDiff {
            x: DaRegister::new_set(8),
            y: DaRegister::new_unset(),
        };

        let p23 = DaPointDiff {
            x: DaRegister::new_set(8),
            y: DaRegister::new_set(16),
        };

        let mut p1c = p1;
        p12.apply(&mut p1c).unwrap();
        assert_eq!(p1c, p2);

        let mut p1c = p1;
        p13.apply(&mut p1c).unwrap();
        assert_eq!(p1c, p3);

        let mut p2c = p2;
        p23.apply(&mut p2c).unwrap();
        assert_eq!(p2c, p3);
    }

    // Test type coercion feature
    mod coercion {
        use crate::{
            ContextlessDaWrite, DaCounter, DaRegister, counter_schemes::CtrU64ByU8,
            decode_buf_exact, encode_to_vec,
        };

        /// Wrapper type that implements Into<i32>
        #[derive(Clone, Copy, Debug, Default)]
        struct WrappedI32(i32);

        impl From<WrappedI32> for i32 {
            fn from(w: WrappedI32) -> i32 {
                w.0
            }
        }

        impl crate::Codec for WrappedI32 {
            fn encode(&self, enc: &mut impl crate::Encoder) -> Result<(), crate::CodecError> {
                self.0.encode(enc)
            }

            fn decode(dec: &mut impl crate::Decoder) -> Result<Self, crate::CodecError> {
                Ok(Self(i32::decode(dec)?))
            }
        }

        /// Target type with raw i32 fields
        #[derive(Copy, Clone, Eq, PartialEq, Debug)]
        pub struct Account {
            balance: i32,
            nonce: u64,
        }

        /// Diff type with wrapper for balance and counter for nonce
        #[derive(Debug, Default)]
        pub struct AccountDiff {
            balance: DaRegister<WrappedI32>,
            nonce: DaCounter<CtrU64ByU8>,
        }

        make_compound_impl! {
            AccountDiff u8 => Account {
                balance: register [WrappedI32 => i32],
                nonce: counter (CtrU64ByU8),
            }
        }

        #[test]
        fn test_coercion_apply() {
            let a1 = Account {
                balance: 100,
                nonce: 5,
            };

            let diff = AccountDiff {
                balance: DaRegister::new_set(WrappedI32(200)),
                nonce: DaCounter::new_changed(3),
            };

            let mut a1c = a1;
            diff.apply(&mut a1c).unwrap();
            assert_eq!(a1c.balance, 200);
            assert_eq!(a1c.nonce, 8);
        }

        #[test]
        fn test_coercion_encode_decode() {
            let diff = AccountDiff {
                balance: DaRegister::new_set(WrappedI32(500)),
                nonce: DaCounter::new_changed(10),
            };

            let encoded = encode_to_vec(&diff).expect("encode");
            let decoded: AccountDiff = decode_buf_exact(&encoded).expect("decode");

            assert_eq!(decoded.balance.new_value().unwrap().0, 500);
            assert_eq!(decoded.nonce.diff(), Some(&10u8));
        }
    }
}
