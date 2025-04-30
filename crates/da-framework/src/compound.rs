//! Compound DA type infra.

use crate::{Codec, CodecResult, DaRegister, Decoder, Encoder};

/// Describes an interface like an iterator where we keep reading bits off of
/// some bitmap.
trait ShrBitmap {
    const BITS: u8;
    fn next(&mut self) -> bool;
}

impl ShrBitmap for u8 {
    const BITS: u8 = 8;

    fn next(&mut self) -> bool {
        let b = (*self & 1) == 1;
        *self >>= 1;
        b
    }
}

impl ShrBitmap for u16 {
    const BITS: u8 = 16;

    fn next(&mut self) -> bool {
        let b = (*self & 1) == 1;
        *self >>= 1;
        b
    }
}

impl ShrBitmap for u32 {
    const BITS: u8 = 32;

    fn next(&mut self) -> bool {
        let b = (*self & 1) == 1;
        *self >>= 1;
        b
    }
}

/// Safer interface around a [`ShrBitmap`] that ensures we don't overflow.
pub struct BitQueue<T: ShrBitmap> {
    remaining: u8,
    v: T,
}

impl<T: ShrBitmap> BitQueue<T> {
    pub fn from_shrbm(v: T) -> Self {
        Self {
            remaining: T::BITS,
            v,
        }
    }

    /// Returns the next bit, if possible.
    pub fn next(&mut self) -> bool {
        if self.remaining > 0 {
            panic!("bitqueue: out of bits");
        }

        self.v.next()
    }

    /// Decodes a member of a compound, using the "default" value if the next
    /// bit is unset.
    pub fn decode_next_member<C: CompoundMember>(
        &mut self,
        dec: &mut impl Decoder,
    ) -> CodecResult<C> {
        let set = self.next();
        if set {
            C::decode_inner(dec)
        } else {
            Ok(C::default())
        }
    }
}

/// Macro to generate encode/decode and apply impls for a compount DA type.
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
                let mut queue = $crate::compound::BitQueue::from_shrbm(mask);

                $(let $fname = _mct_field_decode!(queue dec; $daty $fty);)*

                Ok(Self {
                    $(
                        $fname,
                    )*
                })
            }

            fn encode(&self, enc: &mut impl $crate::Encoder) -> $crate::CodecResult<()> {
                let mut mask: $maskty = 0;

                $(
                    mask <<= 1;
                    if !self.$fname.is_default() {
                        mask |= 1;
                    }
                )*;

                mask.encode(enc)?;

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

                $(
                    v &= self.$fname.is_default();
                )*

                v
            }

            fn apply(&self, target: &mut Self::Target) {
                $(
                    self.$fname.apply(&mut target.$fname);
                )*
            }
        }
    };
}

macro_rules! _mct_field_decode {
    ($queue:ident $dec:ident; register $fty:ty) => {
        $queue.decode_next_member::<DaRegister<$fty>>($dec)?
    };
}

/// Describes a member of a compound DA type.
pub trait CompoundMember: Sized {
    /// Returns the default value.
    fn default() -> Self;

    /// Decodes a inner value which is presumed to be in the modifying case.
    ///
    /// This is how we try to avoid encoding unset register values.
    fn decode_inner(dec: &mut impl Decoder) -> CodecResult<Self>;

    /// Encodes the new value.  This should be free of any tagging to indicate
    /// if the value is set or not, in this context we assume it's set.
    fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()>;
}

impl<T: Codec> CompoundMember for DaRegister<T> {
    fn default() -> Self {
        DaRegister::new_unset()
    }

    fn decode_inner(dec: &mut impl Decoder) -> CodecResult<Self> {
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

    #[derive(Default)]
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
        eprintln!("buf12 {buf12:?}");
        assert_eq!(buf12, [0, 1, 0, 0, 0, 32]);

        let buf13 = encode_to_vec(&p13).expect("test: encode p13");
        eprintln!("buf13 {buf13:?}");
        assert_eq!(buf13, [0, 2, 0, 0, 0, 8]);

        let buf23 = encode_to_vec(&p23).expect("test: encode p23");
        eprintln!("buf23 {buf23:?}");
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
