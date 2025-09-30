//! Codecable vec with varint length tag.
//!
//! The varints are optimized to short lengths, since most payloads will be
//! small.  Below are the permitted layouts, always encoded big-endian.
//!
//! ```
//! 0bbbbbbb
//! 10bbbbbb_bbbbbbbb
//! 11bbbbbb_bbbbbbbb_bbbbbbbb_bbbbbbbb
//! ```

use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// The max value one of these varints can have, which is about 1 billion.
pub const VARINT_MAX: u32 = 0x3fffffff;

type VarintInner = u32;

/// Internal varint type.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct Varint(VarintInner);

impl Varint {
    fn new_unchecked(v: VarintInner) -> Self {
        Self(v)
    }

    fn new(v: VarintInner) -> Option<Self> {
        if v > VARINT_MAX {
            return None;
        }

        Some(Self(v as VarintInner))
    }

    fn new_usize(v: usize) -> Option<Self> {
        if v > VARINT_MAX as usize {
            return None;
        }

        Some(Self(v as VarintInner))
    }

    fn inner(self) -> VarintInner {
        self.0
    }

    fn width(&self) -> VarintWidth {
        if self.0 < 128 {
            VarintWidth::U8
        } else if self.0 < 16384 {
            VarintWidth::U16
        } else {
            VarintWidth::U32
        }
    }

    /// # Panics
    ///
    /// If out of bounds.
    fn sanity_check(&self) {
        assert!(self.0 <= VARINT_MAX, "varint_vec: varint out of bounds");
    }
}

impl Codec for Varint {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let first_byte = u8::decode(dec)?;

        let value = match first_byte >> 6 {
            // 0b00xxxxxx or 0b01xxxxxx: single byte encoding
            0 | 1 => first_byte as u32,

            // 0b10xxxxxx: two-byte encoding
            2 => {
                let second_byte = u8::decode(dec)?;
                let bytes = [first_byte & 0x3f, second_byte];
                u16::from_be_bytes(bytes) as u32
            }

            // 0b11xxxxxx: four-byte encoding
            3 => {
                let mut bytes = [first_byte & 0x3f, 0, 0, 0];
                dec.read_buf(&mut bytes[1..4])?;
                u32::from_be_bytes(bytes)
            }

            _ => unreachable!(),
        };

        let vi = Varint::new_unchecked(value);

        #[cfg(test)]
        vi.sanity_check();

        Ok(vi)
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        #[cfg(test)]
        self.sanity_check();

        match self.width() {
            VarintWidth::U8 => (self.0 as u8).encode(enc),
            VarintWidth::U16 => {
                let val = (self.0 as u16) | 0x8000;
                let bytes = val.to_be_bytes();
                enc.write_buf(&bytes)
            }
            VarintWidth::U32 => {
                let val = self.0 | 0xc0000000;
                let bytes = val.to_be_bytes();
                enc.write_buf(&bytes)
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum VarintWidth {
    U8,
    U16,
    U32,
}

/// Vec that ensures capacity stays within bounds of a simple varint.  In
/// practice, this means it has a max capacity of 0x3fffffff, or about 1
/// billion.  It will never reach this size for our purposes.  This
/// exposes most of the same functions as `Vec` does, but with the bounds
/// checking needed to ensure we stay under this size limit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VarVec<T> {
    inner: Vec<T>,
}

impl<T> VarVec<T> {
    /// Convenience function to construct a new instance without doing the
    /// bounds checking.
    fn new_unchecked(inner: Vec<T>) -> Self {
        Self { inner }
    }

    /// Constructs a new empty varvec.
    pub fn new() -> Self {
        Self::new_unchecked(Vec::new())
    }

    /// Constructs a new empty varvec with enough preallocated space to store
    /// the provided number of entries, if it's in bounds.
    pub fn with_capacity(capacity: usize) -> Option<Self> {
        if capacity > VARINT_MAX as usize {
            return None;
        }

        Some(Self::new_unchecked(Vec::with_capacity(capacity)))
    }

    /// Constructs a new empty varvec by wrapping another vec, but only if it's
    /// in bounds.
    pub fn from_vec(inner: Vec<T>) -> Option<Self> {
        if inner.len() > VARINT_MAX as usize {
            return None;
        }

        Some(Self::new_unchecked(inner))
    }

    pub fn inner(&self) -> &[T] {
        &self.inner
    }

    pub fn into_inner(self) -> Vec<T> {
        self.inner
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    fn len_varint(&self) -> Varint {
        Varint::new_usize(self.inner().len()).expect("varint_vec: internal vec oversized")
    }

    pub fn is_empty(&self) -> bool {
        self.inner().is_empty()
    }

    /// Pushes a new element, if there's space for it.
    pub fn push(&mut self, v: T) -> bool {
        if self.inner.len() + 1 > VARINT_MAX as usize {
            return false;
        }

        self.inner.push(v);
        true
    }

    /// Pushes a new element by calling a constructor fn, if there's space for
    /// it.
    pub fn push_with(&mut self, f: impl Fn() -> T) -> bool {
        if self.inner.len() + 1 > VARINT_MAX as usize {
            return false;
        }

        self.inner.push(f());
        true
    }

    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    pub fn reserve(&mut self, additional: usize) -> bool {
        if self.inner.len() + additional > VARINT_MAX as usize {
            return false;
        }

        self.inner.reserve(additional);
        true
    }

    pub fn truncate(&mut self, len: usize) {
        self.inner.truncate(len);
    }

    pub fn resize(&mut self, new_len: usize, value: T) -> bool
    where
        T: Clone,
    {
        if new_len > VARINT_MAX as usize {
            return false;
        }

        self.inner.resize(new_len, value);
        true
    }

    pub fn resize_with<F>(&mut self, new_len: usize, f: F) -> bool
    where
        F: FnMut() -> T,
    {
        if new_len > VARINT_MAX as usize {
            return false;
        }

        self.inner.resize_with(new_len, f);
        true
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.inner.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.inner.get_mut(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.inner.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.inner.iter_mut()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.inner
    }

    pub fn as_slice_mut(&mut self) -> &mut [T] {
        &mut self.inner
    }
}

impl<T> Default for VarVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> std::ops::Deref for VarVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> std::ops::DerefMut for VarVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T> AsRef<[T]> for VarVec<T> {
    fn as_ref(&self) -> &[T] {
        &self.inner
    }
}

impl<T> AsMut<[T]> for VarVec<T> {
    fn as_mut(&mut self) -> &mut [T] {
        &mut self.inner
    }
}

impl<T: Codec> Codec for VarVec<T> {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = Varint::decode(dec)?;
        let len_usize = len.inner() as usize;

        let mut vec = Vec::with_capacity(len_usize);
        for _ in 0..len_usize {
            vec.push(T::decode(dec)?);
        }

        Ok(Self { inner: vec })
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.len_varint().encode(enc)?;

        for item in &self.inner {
            item.encode(enc)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Most of these tests were written by Claude.

    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    /*
        // Test function to check LLVM optimization of VarVec<u8> decode.
        // This should be monomorphized and we can examine its assembly.
        #[inline(never)]
        pub fn decode_u8_varvec_from_slice(data: &[u8]) -> Result<VarVec<u8>, CodecError> {
            let mut decoder = BufDecoder::new(data);
            VarVec::<u8>::decode(&mut decoder)
        }

        #[test]
        fn test_decode_u8_optimization() {
            // Create a reasonably sized test vector to decode
            let data = vec![42u8; 100];
            let mut encoded = Vec::new();
            let varvec = VarVec::from_vec(data.clone()).unwrap();
            varvec.encode(&mut encoded).unwrap();

            // Use the test function
            let result = decode_u8_varvec_from_slice(&encoded).unwrap();
            assert_eq!(result.inner(), &data[..]);
        }
    */

    #[test]
    fn test_varint_new() {
        assert!(Varint::new(0).is_some());
        assert!(Varint::new(127).is_some());
        assert!(Varint::new(128).is_some());
        assert!(Varint::new(16383).is_some());
        assert!(Varint::new(16384).is_some());
        assert!(Varint::new(VARINT_MAX).is_some());
        assert!(Varint::new(VARINT_MAX + 1).is_none());
    }

    #[test]
    fn test_varint_width() {
        assert_eq!(Varint::new(0).unwrap().width(), VarintWidth::U8);
        assert_eq!(Varint::new(127).unwrap().width(), VarintWidth::U8);
        assert_eq!(Varint::new(128).unwrap().width(), VarintWidth::U16);
        assert_eq!(Varint::new(16383).unwrap().width(), VarintWidth::U16);
        assert_eq!(Varint::new(16384).unwrap().width(), VarintWidth::U32);
        assert_eq!(Varint::new(VARINT_MAX).unwrap().width(), VarintWidth::U32);
    }

    #[test]
    fn test_varint_encode_decode_u8() {
        for val in [0u32, 1, 42, 127] {
            let varint = Varint::new(val).unwrap();
            let buf = encode_to_vec(&varint).unwrap();

            assert_eq!(buf.len(), 1, "U8 varint should be 1 byte");

            let decoded: Varint = decode_buf_exact(&buf).unwrap();
            assert_eq!(decoded.inner(), val);
        }
    }

    #[test]
    fn test_varint_encode_decode_u16() {
        for val in [128u32, 200, 1000, 16383] {
            let varint = Varint::new(val).unwrap();
            let buf = encode_to_vec(&varint).unwrap();

            assert_eq!(buf.len(), 2, "U16 varint should be 2 bytes");
            assert_eq!(buf[0] >> 6, 2, "U16 varint should start with 0b10");

            let decoded: Varint = decode_buf_exact(&buf).unwrap();
            assert_eq!(decoded.inner(), val);
        }
    }

    #[test]
    fn test_varint_encode_decode_u32() {
        for val in [16384u32, 100000, 1000000, VARINT_MAX] {
            let varint = Varint::new(val).unwrap();
            let buf = encode_to_vec(&varint).unwrap();

            assert_eq!(buf.len(), 4, "U32 varint should be 4 bytes");
            assert_eq!(buf[0] >> 6, 3, "U32 varint should start with 0b11");

            let decoded: Varint = decode_buf_exact(&buf).unwrap();
            assert_eq!(decoded.inner(), val);
        }
    }

    #[test]
    fn test_varint_boundaries() {
        // Test boundary values
        let boundaries = [0, 127, 128, 16383, 16384, VARINT_MAX];

        for val in boundaries {
            let varint = Varint::new(val).unwrap();
            let buf = encode_to_vec(&varint).unwrap();

            let decoded: Varint = decode_buf_exact(&buf).unwrap();
            assert_eq!(decoded.inner(), val);
        }
    }

    #[test]
    fn test_varvec_new() {
        let vec: VarVec<u32> = VarVec::new();
        assert!(vec.is_empty());
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn test_varvec_from_vec() {
        let inner = vec![1u32, 2, 3, 4, 5];
        let varvec = VarVec::from_vec(inner.clone()).unwrap();
        assert_eq!(varvec.len(), 5);
        assert_eq!(varvec.inner(), &inner[..]);
    }

    #[test]
    fn test_varvec_push_pop() {
        let mut vec: VarVec<u32> = VarVec::new();
        assert!(vec.push(1));
        assert!(vec.push(2));
        assert!(vec.push(3));

        assert_eq!(vec.len(), 3);
        assert_eq!(vec.pop(), Some(3));
        assert_eq!(vec.pop(), Some(2));
        assert_eq!(vec.pop(), Some(1));
        assert_eq!(vec.pop(), None);
        assert!(vec.is_empty());
    }

    #[test]
    fn test_varvec_clear() {
        let mut vec = VarVec::from_vec(vec![1u32, 2, 3]).unwrap();
        assert!(!vec.is_empty());
        vec.clear();
        assert!(vec.is_empty());
    }

    #[test]
    fn test_varvec_truncate() {
        let mut vec = VarVec::from_vec(vec![1u32, 2, 3, 4, 5]).unwrap();
        vec.truncate(3);
        assert_eq!(vec.len(), 3);
        assert_eq!(vec.inner(), &[1, 2, 3]);
    }

    #[test]
    fn test_varvec_encode_decode_empty() {
        let vec: VarVec<u32> = VarVec::new();
        let buf = encode_to_vec(&vec).unwrap();

        let decoded: VarVec<u32> = decode_buf_exact(&buf).unwrap();
        assert_eq!(decoded.len(), 0);
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_varvec_encode_decode_small() {
        let vec = VarVec::from_vec(vec![1u32, 2, 3]).unwrap();
        let buf = encode_to_vec(&vec).unwrap();

        let decoded: VarVec<u32> = decode_buf_exact(&buf).unwrap();
        assert_eq!(decoded.inner(), vec.inner());
    }

    #[test]
    fn test_varvec_encode_decode_u8() {
        let vec = VarVec::from_vec(vec![1u8, 2, 3, 255]).unwrap();
        let buf = encode_to_vec(&vec).unwrap();

        let decoded: VarVec<u8> = decode_buf_exact(&buf).unwrap();
        assert_eq!(decoded.inner(), vec.inner());
    }

    #[test]
    fn test_varvec_encode_decode_large_len() {
        // Test with length that requires 2-byte varint
        let data = vec![42u8; 200];
        let vec = VarVec::from_vec(data.clone()).unwrap();
        let buf = encode_to_vec(&vec).unwrap();

        let decoded: VarVec<u8> = decode_buf_exact(&buf).unwrap();
        assert_eq!(decoded.len(), 200);
        assert_eq!(decoded.inner(), &data[..]);
    }

    #[test]
    fn test_varvec_with_capacity() {
        let vec: VarVec<u32> = VarVec::with_capacity(10).unwrap();
        assert!(vec.is_empty());
        assert!(vec.capacity() >= 10);
    }

    #[test]
    fn test_varvec_reserve() {
        let mut vec: VarVec<u32> = VarVec::new();
        assert!(vec.reserve(100));
        assert!(vec.capacity() >= 100);
    }

    #[test]
    fn test_varvec_max_size_check() {
        // Verify that attempting to create a VarVec larger than VARINT_MAX fails
        let large_size = (VARINT_MAX as usize) + 1;
        assert!(VarVec::<u8>::with_capacity(large_size).is_none());
    }

    #[test]
    fn test_varvec_into_inner() {
        let data = vec![1u32, 2, 3];
        let vec = VarVec::from_vec(data.clone()).unwrap();
        let inner = vec.into_inner();
        assert_eq!(inner, data);
    }

    #[test]
    fn test_varvec_resize() {
        let mut vec = VarVec::from_vec(vec![1u32, 2, 3]).unwrap();
        assert!(vec.resize(5, 99));
        assert_eq!(vec.len(), 5);
        assert_eq!(vec.inner(), &[1, 2, 3, 99, 99]);

        assert!(vec.resize(2, 0));
        assert_eq!(vec.len(), 2);
        assert_eq!(vec.inner(), &[1, 2]);
    }

    #[test]
    fn test_varvec_resize_with() {
        let mut vec = VarVec::from_vec(vec![1u32, 2, 3]).unwrap();
        let mut counter = 10;
        assert!(vec.resize_with(5, || {
            counter += 1;
            counter
        }));
        assert_eq!(vec.len(), 5);
        assert_eq!(vec.inner(), &[1, 2, 3, 11, 12]);
    }

    #[test]
    fn test_varvec_resize_too_large() {
        let mut vec: VarVec<u32> = VarVec::new();
        assert!(!vec.resize((VARINT_MAX as usize) + 1, 0));
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn test_varvec_resize_with_too_large() {
        let mut vec: VarVec<u32> = VarVec::new();
        assert!(!vec.resize_with((VARINT_MAX as usize) + 1, || 0));
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn test_varvec_reserve_too_large() {
        let mut vec = VarVec::from_vec(vec![1u32; 100]).unwrap();
        // Try to reserve enough to exceed VARINT_MAX
        assert!(!vec.reserve(VARINT_MAX as usize));
        // Vec should be unchanged
        assert_eq!(vec.len(), 100);
    }

    #[test]
    fn test_varvec_push_at_limit() {
        // Create a VarVec at VARINT_MAX capacity
        let data = vec![42u8; VARINT_MAX as usize];
        let mut vec = VarVec::from_vec(data).unwrap();
        assert_eq!(vec.len(), VARINT_MAX as usize);

        // Pushing should fail
        assert!(!vec.push(99));
        assert_eq!(vec.len(), VARINT_MAX as usize);
    }

    #[test]
    fn test_varvec_push_with_at_limit() {
        let data = vec![42u8; VARINT_MAX as usize];
        let mut vec = VarVec::from_vec(data).unwrap();

        assert!(!vec.push_with(|| 99));
        assert_eq!(vec.len(), VARINT_MAX as usize);
    }

    #[test]
    fn test_varvec_as_slice() {
        use core::slice::SlicePattern;

        let vec = VarVec::from_vec(vec![1u32, 2, 3, 4, 5]).unwrap();
        let slice = vec.as_slice();
        assert_eq!(slice, &[1, 2, 3, 4, 5]);

        // Test that methods like strip_prefix work
        assert_eq!(slice.strip_prefix(&[1, 2]), Some(&[3, 4, 5][..]));
    }
}
