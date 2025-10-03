//! Compound DA type infra.

use crate::{Codec, CodecResult, DaRegister, DaWrite, Decoder, Encoder};

/// Describes a bitmap we can read/write to.
pub trait ShiftBitmap: Copy {
    /// Returns the total number of bits we can store.
    const BITS: u8;

    /// Returns an empty bitmap.
    fn zero() -> Self;

    /// Reads the bit at some some index.
    fn get(&self, off: u8) -> bool;

    /// Writes the bit at some index.
    fn put(&mut self, off: u8, b: bool);
}

macro_rules! impl_shift_bitmap {
    ($t:ident) => {
        impl ShiftBitmap for $t {
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

impl_shift_bitmap!(u8);
impl_shift_bitmap!(u16);
impl_shift_bitmap!(u32);

/// Safer interface around a [`ShiftBitmap`] that ensures we don't overflow.
pub struct BitReader<T: ShiftBitmap> {
    off: u8,
    mask: T,
}

impl<T: ShiftBitmap> BitReader<T> {
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

/// Safer interface around a [`ShiftBitmap`] that ensures we don't overflow.
pub struct BitWriter<T: ShiftBitmap> {
    off: u8,
    mask: T,
}

impl<T: ShiftBitmap> BitWriter<T> {
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
#[macro_export]
macro_rules! make_compound_traits {
    (
        $tyname:ident $maskty:ident => $target:ty {
            $(
                $fname:ident : $daty:ident $fty:ty ,
            )*
        }
    ) => {
        impl $crate::Codec for $tyname {
            fn decode(dec: &mut impl $crate::Decoder) -> $crate::CodecResult<Self> {
                let mask = <$maskty>::decode(dec)?;
                let mut bitr = $crate::compound::BitReader::from_mask(mask);

                $(let $fname = _mct_field_decode!(bitr dec; $daty $fty);)*

                Ok(Self { $($fname,)* })
            }

            fn encode(&self, enc: &mut impl $crate::Encoder) -> $crate::CodecResult<()> {
                let mut bitw = $crate::compound::BitWriter::<$maskty>::new();

                $(bitw.prepare_member(&self.$fname);)*

                bitw.mask().encode(enc)?;

                // This goes through them in the same order as the above, which
                // is why this is safe.
                $(
                    if !self.$fname.is_default() {
                        self.$fname.encode_set(enc)?;
                    }
                )*

                Ok(())
            }
        }

        impl $crate::DaWrite for $tyname {
            type Target = $target;

            fn is_default(&self) -> bool {
                let mut v = true;

                // Kinda weird way to && all these different values.
                $(
                    v &= self.$fname.is_default();
                )*

                v
            }

            fn apply(&self, target: &mut Self::Target) {
                // Depends on all the members being accessible.
                $(
                    self.$fname.apply(&mut target.$fname);
                )*
            }
        }
    };
}

/// Expands to a decoder for each type of member that we support in a compound.
macro_rules! _mct_field_decode {
    // Register
    ($reader:ident $dec:ident; register $fty:ty) => {
        $reader.decode_next_member::<DaRegister<$fty>>($dec)?
    };
}

/// Describes a member of a compound DA type.
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

impl<T: Codec + Clone> CompoundMember for DaRegister<T> {
    fn default() -> Self {
        DaRegister::new_unset()
    }

    fn is_default(&self) -> bool {
        <DaRegister<_> as DaWrite>::is_default(self)
    }

    fn decode_set(dec: &mut impl Decoder) -> CodecResult<Self> {
        DaRegister::set_from_decoder(dec)
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        self.encode_if_set(enc)
    }
}

#[cfg(test)]
mod tests {
    use crate::{DaRegister, DaWrite, encode_to_vec};

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

    make_compound_traits! {
        DaPointDiff u16 => Point {
            x: register i32,
            y: register i32,
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
        p12.apply(&mut p1c);
        assert_eq!(p1c, p2);

        let mut p1c = p1;
        p13.apply(&mut p1c);
        assert_eq!(p1c, p3);

        let mut p2c = p2;
        p23.apply(&mut p2c);
        assert_eq!(p2c, p3);
    }
}
